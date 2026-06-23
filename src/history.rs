use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local, TimeZone, Utc};
use serde_json::Value;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct Conversation {
    pub path: PathBuf,
    pub session_id: String,
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    pub started_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
    pub title: String,
    pub preview: String,
    pub messages: Vec<Message>,
    pub search_text_normalized: String,
    pub title_normalized: String,
    pub cwd_normalized: String,
    pub session_id_normalized: String,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub kind: MessageKind,
    pub timestamp: DateTime<Local>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Message,
    Reasoning,
    ToolCall,
    ToolOutput,
}

#[derive(Debug, Clone)]
pub struct LoadOptions {
    pub codex_home: PathBuf,
    pub current_dir: PathBuf,
    pub show_tools: bool,
    pub show_reasoning: bool,
    pub debug: bool,
}

pub fn codex_home(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path);
    }
    if let Ok(path) = std::env::var("CODEX_HOME")
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    Ok(home::home_dir()
        .context("failed to find home directory")?
        .join(".codex"))
}

pub fn history_titles(codex_home: &Path) -> Result<HashMap<String, String>> {
    let path = codex_home.join("history.jsonl");
    let Ok(file) = File::open(&path) else {
        return Ok(HashMap::new());
    };
    let reader = BufReader::new(file);
    let mut titles = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = value.get("session_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(text) = value.get("text").and_then(Value::as_str) else {
            continue;
        };
        if !text.trim().is_empty() {
            titles
                .entry(id.to_string())
                .or_insert_with(|| one_line(text, 120));
        }
    }
    Ok(titles)
}

pub fn load_conversations(options: &LoadOptions) -> Result<Vec<Conversation>> {
    let titles = history_titles(&options.codex_home)?;
    let sessions_dir = options.codex_home.join("sessions");
    let mut conversations = Vec::new();

    for entry in WalkDir::new(&sessions_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        match load_conversation_for_search(
            entry.path(),
            &titles,
            options.show_tools,
            options.show_reasoning,
        ) {
            Ok(conversation) => conversations.push(conversation),
            Err(err) if options.debug => eprintln!("{}: {err:#}", entry.path().display()),
            Err(_) => {}
        }
    }

    conversations.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(conversations)
}

pub fn load_conversation(path: &Path, titles: &HashMap<String, String>) -> Result<Conversation> {
    load_conversation_for_search(path, titles, true, true)
}

fn load_conversation_for_search(
    path: &Path,
    titles: &HashMap<String, String>,
    include_tools_in_search: bool,
    include_reasoning_in_search: bool,
) -> Result<Conversation> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut session_id = session_id_from_filename(path).unwrap_or_else(|| "unknown".to_string());
    let mut cwd = None;
    let mut model = None;
    let mut started_at = None;
    let mut updated_at = None;
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line)?;
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
            .unwrap_or_else(Local::now);
        started_at =
            Some(started_at.map_or(timestamp, |current: DateTime<Local>| current.min(timestamp)));
        updated_at =
            Some(updated_at.map_or(timestamp, |current: DateTime<Local>| current.max(timestamp)));

        let top_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload = value.get("payload").unwrap_or(&Value::Null);
        if top_type == "session_meta" {
            if let Some(id) = payload.get("id").and_then(Value::as_str) {
                session_id = id.to_string();
            }
            if let Some(path) = payload.get("cwd").and_then(Value::as_str) {
                cwd = Some(PathBuf::from(path));
            }
            continue;
        }
        if top_type == "turn_context" {
            if let Some(path) = payload.get("cwd").and_then(Value::as_str) {
                cwd = Some(PathBuf::from(path));
            }
            if let Some(name) = payload.get("model").and_then(Value::as_str) {
                model = Some(name.to_string());
            }
            continue;
        }

        if top_type != "response_item" {
            continue;
        }

        let payload_type = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match payload_type {
            "message" => {
                let role = payload
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("message")
                    .to_string();
                if role == "system" || role == "developer" {
                    continue;
                }
                let text = extract_content(payload.get("content"));
                if !text.trim().is_empty() {
                    messages.push(Message {
                        role,
                        kind: MessageKind::Message,
                        timestamp,
                        text,
                    });
                }
            }
            "reasoning" => {
                let text = extract_reasoning(payload);
                if !text.trim().is_empty() {
                    messages.push(Message {
                        role: "reasoning".to_string(),
                        kind: MessageKind::Reasoning,
                        timestamp,
                        text,
                    });
                }
            }
            "function_call" => {
                let name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let arguments = payload
                    .get("arguments")
                    .map(value_to_compact_string)
                    .unwrap_or_default();
                messages.push(Message {
                    role: name.to_string(),
                    kind: MessageKind::ToolCall,
                    timestamp,
                    text: arguments,
                });
            }
            "function_call_output" => {
                let text = payload
                    .get("output")
                    .map(value_to_prettyish_string)
                    .unwrap_or_default();
                if !text.trim().is_empty() {
                    messages.push(Message {
                        role: "tool output".to_string(),
                        kind: MessageKind::ToolOutput,
                        timestamp,
                        text,
                    });
                }
            }
            _ => {}
        }
    }

    let started_at = started_at.unwrap_or_else(Local::now);
    let updated_at = updated_at.unwrap_or(started_at);
    let title = titles
        .get(&session_id)
        .cloned()
        .or_else(|| {
            messages
                .iter()
                .find(|m| m.role == "user")
                .map(|m| one_line(&m.text, 120))
        })
        .unwrap_or_else(|| session_id.clone());
    let preview = messages
        .iter()
        .find(|m| m.role == "assistant")
        .or_else(|| messages.iter().find(|m| m.role == "user"))
        .map(|m| one_line(&m.text, 180))
        .unwrap_or_default();
    let mut search_text = String::new();
    search_text.push_str(&session_id);
    search_text.push('\n');
    search_text.push_str(&title);
    search_text.push('\n');
    if let Some(cwd) = &cwd {
        search_text.push_str(&cwd.to_string_lossy());
        search_text.push('\n');
    }
    if let Some(model) = &model {
        search_text.push_str(model);
        search_text.push('\n');
    }
    for message in &messages {
        match message.kind {
            MessageKind::ToolCall | MessageKind::ToolOutput if !include_tools_in_search => {
                continue;
            }
            MessageKind::Reasoning if !include_reasoning_in_search => continue,
            _ => {}
        }
        search_text.push_str(&message.role);
        search_text.push('\n');
        search_text.push_str(&message.text);
        search_text.push('\n');
    }
    let cwd_normalized = cwd
        .as_ref()
        .map(|p| crate::search::normalize(&p.to_string_lossy()))
        .unwrap_or_default();
    let search_text_normalized = crate::search::normalize(&search_text);
    let title_normalized = crate::search::normalize(&title);
    let session_id_normalized = crate::search::normalize(&session_id);

    Ok(Conversation {
        path: path.to_path_buf(),
        session_id,
        cwd,
        model,
        started_at,
        updated_at,
        title,
        preview,
        messages,
        search_text_normalized,
        title_normalized,
        cwd_normalized,
        session_id_normalized,
    })
}

fn parse_timestamp(input: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(input)
        .map(|dt| dt.with_timezone(&Local))
        .ok()
}

fn session_id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    stem.rsplit_once('-')
        .map(|(_, right)| right.to_string())
        .filter(|right| right.len() >= 8)
        .or_else(|| Some(stem.to_string()))
}

fn extract_content(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                if let Some(text) = item.as_str() {
                    return Some(text.to_string());
                }
                item.get("text")
                    .or_else(|| item.get("input_text"))
                    .or_else(|| item.get("output_text"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => value_to_prettyish_string(other),
        None => String::new(),
    }
}

fn extract_reasoning(payload: &Value) -> String {
    if let Some(summary) = payload.get("summary") {
        return match summary {
            Value::String(text) => text.clone(),
            Value::Array(items) => items
                .iter()
                .filter_map(|item| {
                    item.as_str().map(ToString::to_string).or_else(|| {
                        item.get("text")
                            .or_else(|| item.get("summary"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                    })
                })
                .collect::<Vec<_>>()
                .join("\n"),
            other => value_to_prettyish_string(other),
        };
    }
    String::new()
}

fn value_to_compact_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn value_to_prettyish_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn one_line(input: &str, max_chars: usize) -> String {
    let mut out = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
    }
    out
}

#[allow(dead_code)]
fn unix_to_local(ts: i64) -> DateTime<Local> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(&Local)
}

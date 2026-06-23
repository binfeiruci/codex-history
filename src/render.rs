use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::history::{Conversation, MessageKind};

const TIME_WIDTH: usize = 7;
const NAME_WIDTH: usize = 9;
const SEPARATOR: &str = " │ ";

#[derive(Clone)]
pub struct TuiLine {
    pub text: String,
    pub line: Line<'static>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolDisplay {
    Hidden,
    Abbreviated,
    Full,
}

impl ToolDisplay {
    pub fn from_show_tools(show_tools: bool) -> Self {
        if show_tools { Self::Full } else { Self::Hidden }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Hidden => Self::Abbreviated,
            Self::Abbreviated => Self::Full,
            Self::Full => Self::Hidden,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hidden => "off",
            Self::Abbreviated => "brief",
            Self::Full => "on",
        }
    }
}

pub fn print_conversation(conversation: &Conversation, show_tools: bool, show_reasoning: bool) {
    println!("# {}", conversation.title);
    println!("session: {}", conversation.session_id);
    println!("file: {}", conversation.path.display());
    if let Some(cwd) = &conversation.cwd {
        println!("cwd: {}", cwd.display());
    }
    if let Some(model) = &conversation.model {
        println!("model: {model}");
    }
    println!();

    for message in &conversation.messages {
        match message.kind {
            MessageKind::ToolCall | MessageKind::ToolOutput if !show_tools => continue,
            MessageKind::Reasoning if !show_reasoning => continue,
            _ => {}
        }
        println!(
            "## {}  {}",
            label(message.kind, &message.role),
            message.timestamp.format("%Y-%m-%d %H:%M:%S")
        );
        println!("{}", message.text.trim_end());
        println!();
    }
}

pub fn render_tui_lines(
    conversation: &Conversation,
    width: u16,
    tool_display: ToolDisplay,
    show_reasoning: bool,
) -> Vec<TuiLine> {
    let content_width = usize::from(width)
        .saturating_sub(TIME_WIDTH + NAME_WIDTH + SEPARATOR.len() + 4)
        .max(24);
    let mut lines = Vec::new();

    lines.push(metadata_line(vec![
        ("session ".to_string(), muted_style()),
        (conversation.session_id.clone(), secondary_style()),
    ]));
    if let Some(cwd) = &conversation.cwd {
        lines.push(metadata_line(vec![
            ("cwd     ".to_string(), muted_style()),
            (cwd.display().to_string(), secondary_style()),
        ]));
    }
    if let Some(model) = &conversation.model {
        lines.push(metadata_line(vec![
            ("model   ".to_string(), muted_style()),
            (model.clone(), secondary_style()),
        ]));
    }
    lines.push(TuiLine::blank());

    for message in &conversation.messages {
        match message.kind {
            MessageKind::ToolCall | MessageKind::ToolOutput
                if tool_display == ToolDisplay::Hidden =>
            {
                continue;
            }
            MessageKind::Reasoning if !show_reasoning => continue,
            _ => {}
        }

        let spec = MessageSpec::from_message(message.kind, &message.role);
        let timestamp = message.timestamp.format("%H:%M").to_string();
        let text;
        let content_text = if message.kind == MessageKind::ToolOutput
            && tool_display == ToolDisplay::Abbreviated
        {
            text = abbreviated_tool_output(&message.text);
            text.as_str()
        } else {
            message.text.as_str()
        };
        let content = markdown_lines(content_text, content_width, spec.body_style);
        if content.is_empty() {
            lines.push(ledger_line(
                Some(&timestamp),
                &spec.label,
                spec.label_style,
                Vec::new(),
            ));
        } else {
            for (line_idx, content_spans) in content.into_iter().enumerate() {
                lines.push(ledger_line(
                    (line_idx == 0).then_some(timestamp.as_str()),
                    if line_idx == 0 { &spec.label } else { "" },
                    spec.label_style,
                    content_spans,
                ));
            }
        }
        lines.push(TuiLine::blank());
    }

    lines
}

fn abbreviated_tool_output(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "(empty tool output)".to_string();
    }

    let first_line = trimmed
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let preview = truncate_chars(&first_line, 180);
    let line_count = trimmed.lines().count();
    let char_count = trimmed.chars().count();
    if line_count <= 1 && char_count <= preview.chars().count() {
        preview
    } else {
        format!("{preview} ... (tool output omitted: {line_count} lines, {char_count} chars)")
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out = input
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

pub fn highlighted_line(line: &TuiLine, query: &str) -> Line<'static> {
    let needle = query.trim();
    if needle.is_empty()
        || !line
            .text
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    {
        return line.line.clone();
    }

    let lowered = line.text.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut cursor = 0;
    for (start, _) in lowered.match_indices(&needle) {
        if start > cursor {
            spans.push(Span::styled(
                line.text[cursor..start].to_string(),
                Style::default().fg(Color::Gray),
            ));
        }
        spans.push(Span::styled(
            line.text[start..start + needle.len()].to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(78, 201, 176))
                .add_modifier(Modifier::BOLD),
        ));
        cursor = start + needle.len();
    }
    if cursor < line.text.len() {
        spans.push(Span::styled(
            line.text[cursor..].to_string(),
            Style::default().fg(Color::Gray),
        ));
    }

    Line::from(spans)
}

fn label(kind: MessageKind, role: &str) -> String {
    match kind {
        MessageKind::Message => role.to_uppercase(),
        MessageKind::Reasoning => "REASONING".to_string(),
        MessageKind::ToolCall => format!("TOOL {}", role),
        MessageKind::ToolOutput => "TOOL OUTPUT".to_string(),
    }
}

struct MessageSpec {
    label: String,
    label_style: Style,
    body_style: Style,
}

impl MessageSpec {
    fn from_message(kind: MessageKind, role: &str) -> Self {
        match kind {
            MessageKind::Message if role == "user" => Self {
                label: "You".to_string(),
                label_style: Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
                body_style: Style::default().fg(Color::White),
            },
            MessageKind::Message => Self {
                label: "Codex".to_string(),
                label_style: Style::default()
                    .fg(Color::Rgb(78, 201, 176))
                    .add_modifier(Modifier::BOLD),
                body_style: Style::default().fg(Color::White),
            },
            MessageKind::Reasoning => Self {
                label: "Thinking".to_string(),
                label_style: Style::default().fg(Color::Rgb(140, 145, 150)),
                body_style: Style::default()
                    .fg(Color::Rgb(140, 145, 150))
                    .add_modifier(Modifier::ITALIC),
            },
            MessageKind::ToolCall => Self {
                label: short_tool_label(role),
                label_style: Style::default().fg(Color::Rgb(140, 145, 150)),
                body_style: Style::default().fg(Color::Rgb(140, 145, 150)),
            },
            MessageKind::ToolOutput => Self {
                label: "Result".to_string(),
                label_style: Style::default().fg(Color::Rgb(140, 145, 150)),
                body_style: Style::default().fg(Color::Rgb(140, 145, 150)),
            },
        }
    }
}

impl TuiLine {
    fn blank() -> Self {
        Self {
            text: String::new(),
            line: Line::from(""),
        }
    }
}

fn metadata_line(spans: Vec<(String, Style)>) -> TuiLine {
    let text = spans.iter().map(|(text, _)| text.as_str()).collect();
    let line = Line::from(
        spans
            .into_iter()
            .map(|(text, style)| Span::styled(text, style))
            .collect::<Vec<_>>(),
    );
    TuiLine { text, line }
}

fn ledger_line(
    timestamp: Option<&str>,
    label: &str,
    label_style: Style,
    content: Vec<(String, Style)>,
) -> TuiLine {
    let mut spans = Vec::new();
    spans.push(Span::styled(
        timestamp
            .map(|ts| format!(" {ts} "))
            .unwrap_or_else(|| " ".repeat(TIME_WIDTH)),
        muted_style(),
    ));
    spans.push(Span::styled(
        format!("{label:>NAME_WIDTH$}"),
        if label.is_empty() {
            Style::default()
        } else {
            label_style
        },
    ));
    spans.push(Span::styled(SEPARATOR, border_style()));
    spans.extend(
        content
            .iter()
            .map(|(text, style)| Span::styled(text.clone(), *style)),
    );
    let text = spans.iter().map(|span| span.content.as_ref()).collect();
    TuiLine {
        text,
        line: Line::from(spans),
    }
}

fn markdown_lines(input: &str, width: usize, base_style: Style) -> Vec<Vec<(String, Style)>> {
    let mut lines = Vec::new();
    let mut in_code = false;

    for raw in input.lines() {
        let trimmed = raw.trim_end();
        if trimmed.trim().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if trimmed.trim().is_empty() {
            lines.push(Vec::new());
            continue;
        }

        let (text, style) = if in_code {
            (
                trimmed.to_string(),
                base_style.fg(Color::Rgb(147, 161, 199)),
            )
        } else if let Some(heading) = trimmed.trim_start().strip_prefix("# ") {
            (
                heading.trim().to_string(),
                Style::default()
                    .fg(Color::Rgb(180, 190, 200))
                    .add_modifier(Modifier::BOLD),
            )
        } else if let Some(heading) = trimmed.trim_start().strip_prefix("## ") {
            (
                heading.trim().to_string(),
                Style::default()
                    .fg(Color::Rgb(180, 190, 200))
                    .add_modifier(Modifier::BOLD),
            )
        } else if let Some(quote) = trimmed.trim_start().strip_prefix(">") {
            (
                format!(">{}", quote),
                base_style.fg(Color::Rgb(120, 200, 120)),
            )
        } else {
            (trimmed.to_string(), base_style)
        };

        let runs = if in_code {
            vec![(text, style)]
        } else {
            inline_markdown_runs(&text, style)
        };
        lines.extend(wrap_runs(runs, width));
    }

    lines
}

fn inline_markdown_runs(input: &str, base_style: Style) -> Vec<(String, Style)> {
    let parser = Parser::new_ext(input, Options::empty());
    let mut styles = vec![base_style];
    let mut runs = Vec::new();

    for event in parser {
        match event {
            Event::Text(text) => runs.push((text.to_string(), *styles.last().unwrap())),
            Event::Code(text) => runs.push((
                text.to_string(),
                styles
                    .last()
                    .copied()
                    .unwrap_or(base_style)
                    .fg(Color::Rgb(147, 161, 199))
                    .add_modifier(Modifier::BOLD),
            )),
            Event::SoftBreak | Event::HardBreak => runs.push((" ".to_string(), base_style)),
            Event::Start(tag) => {
                let next = match tag {
                    Tag::Strong => styles
                        .last()
                        .copied()
                        .unwrap_or(base_style)
                        .add_modifier(Modifier::BOLD),
                    Tag::Emphasis => styles
                        .last()
                        .copied()
                        .unwrap_or(base_style)
                        .add_modifier(Modifier::ITALIC),
                    Tag::Link { .. } => styles
                        .last()
                        .copied()
                        .unwrap_or(base_style)
                        .fg(Color::Rgb(100, 149, 237)),
                    _ => styles.last().copied().unwrap_or(base_style),
                };
                styles.push(next);
            }
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link) => {
                if styles.len() > 1 {
                    styles.pop();
                }
            }
            _ => {}
        }
    }

    if runs.is_empty() {
        vec![(input.to_string(), base_style)]
    } else {
        runs
    }
}

fn wrap_runs(runs: Vec<(String, Style)>, width: usize) -> Vec<Vec<(String, Style)>> {
    let mut lines = vec![Vec::new()];
    let mut current_width = 0;

    for (text, style) in runs {
        for segment in split_segments(&text) {
            if segment == "\n" {
                lines.push(Vec::new());
                current_width = 0;
                continue;
            }

            let segment_width = UnicodeWidthStr::width(segment.as_str());
            if segment.trim().is_empty() {
                if current_width == 0 {
                    continue;
                }
                if current_width + segment_width <= width {
                    push_span(lines.last_mut().unwrap(), segment, style);
                    current_width += segment_width;
                }
                continue;
            }

            if current_width > 0 && current_width + segment_width > width {
                lines.push(Vec::new());
                current_width = 0;
            }

            if segment_width <= width {
                push_span(lines.last_mut().unwrap(), segment, style);
                current_width += segment_width;
            } else {
                for ch in segment.chars() {
                    let char_width = ch.width().unwrap_or(0);
                    if current_width > 0 && current_width + char_width > width {
                        lines.push(Vec::new());
                        current_width = 0;
                    }
                    push_span(lines.last_mut().unwrap(), ch.to_string(), style);
                    current_width += char_width;
                }
            }
        }
    }

    lines
}

fn split_segments(input: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_whitespace = None;

    for ch in input.chars() {
        if ch == '\n' {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            segments.push("\n".to_string());
            current_whitespace = None;
            continue;
        }

        let whitespace = ch.is_whitespace();
        match current_whitespace {
            Some(prev) if prev == whitespace => current.push(ch),
            Some(_) => {
                segments.push(std::mem::take(&mut current));
                current.push(ch);
                current_whitespace = Some(whitespace);
            }
            None => {
                current.push(ch);
                current_whitespace = Some(whitespace);
            }
        }
    }

    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn push_span(line: &mut Vec<(String, Style)>, text: String, style: Style) {
    if let Some((last_text, last_style)) = line.last_mut()
        && *last_style == style
    {
        last_text.push_str(&text);
        return;
    }
    line.push((text, style));
}

fn short_tool_label(role: &str) -> String {
    let name = role
        .rsplit_once('.')
        .map(|(_, right)| right)
        .unwrap_or(role);
    let mut out = name.chars().take(NAME_WIDTH).collect::<String>();
    if out.is_empty() {
        out = "Tool".to_string();
    }
    out
}

fn muted_style() -> Style {
    Style::default().fg(Color::Rgb(100, 100, 100))
}

fn secondary_style() -> Style {
    Style::default().fg(Color::Rgb(140, 140, 140))
}

fn border_style() -> Style {
    Style::default().fg(Color::Rgb(60, 60, 60))
}

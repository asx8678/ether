// Modified from OpenAI Codex (codex-rs/tui/src/git_action_directives.rs @ 806d973): removed unit tests.
//! Codex App directives embedded in assistant markdown.
//!
//! Preserve whitespace on unchanged lines: Markdown hard breaks and code can depend on it.

use crate::assistant_directives::AssistantDirective;
use crate::assistant_directives::QuoteEscaping;
use crate::assistant_directives::parse_assistant_directive;
use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum GitActionDirective {
    Stage {
        cwd: String,
    },
    Commit {
        cwd: String,
    },
    CreateBranch {
        cwd: String,
        branch: String,
    },
    Push {
        cwd: String,
        branch: String,
    },
    CreatePr {
        cwd: String,
        branch: String,
        url: Option<String>,
        is_draft: bool,
    },
}

impl GitActionDirective {
    pub(crate) fn created_branch_cwd(&self) -> Option<&str> {
        match self {
            Self::CreateBranch { cwd, .. } => Some(cwd),
            _ => None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ParsedAssistantMarkdown {
    pub(crate) visible_markdown: String,
    pub(crate) git_actions: Vec<GitActionDirective>,
}

impl ParsedAssistantMarkdown {
    pub(crate) fn last_created_branch_cwd(&self) -> Option<&str> {
        self.git_actions
            .iter()
            .rev()
            .find_map(GitActionDirective::created_branch_cwd)
    }
}

pub(crate) fn parse_assistant_markdown(markdown: &str, cwd: &Path) -> ParsedAssistantMarkdown {
    let mut git_actions = Vec::new();
    let mut seen = HashSet::new();
    let mut visible_lines = Vec::new();

    for line in markdown.lines() {
        let (visible_line, line_actions) =
            rewrite_code_comment_line(line, cwd).unwrap_or_else(|| strip_line_directives(line));
        for action in line_actions {
            if seen.insert(action.clone()) {
                git_actions.push(action);
            }
        }
        visible_lines.push(if visible_line == line {
            visible_line
        } else {
            visible_line.trim_end().to_string()
        });
    }

    while visible_lines
        .last()
        .is_some_and(|line| line.trim().is_empty())
    {
        visible_lines.pop();
    }

    ParsedAssistantMarkdown {
        visible_markdown: visible_lines.join("\n"),
        git_actions,
    }
}

fn rewrite_code_comment_line(line: &str, cwd: &Path) -> Option<(String, Vec<GitActionDirective>)> {
    let content = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - content.len()];
    let directive = parse_assistant_directive(content, QuoteEscaping::Backslash)?;
    if directive.name != "code-comment" {
        return None;
    }
    let suffix = &content[directive.raw.len()..];
    let title = directive.attributes.get("title")?;
    let body = directive.attributes.get("body")?;
    let file = directive.attributes.get("file")?;
    let title = title.trim();
    let body = body.trim();
    let file = file.trim();
    (!title.is_empty() && !body.is_empty() && !file.is_empty()).then_some(())?;

    let start = directive_integer(&directive, "start").unwrap_or(1).max(1);
    let end = directive_integer(&directive, "end")
        .unwrap_or(start)
        .max(start);
    let title = if title_has_priority(title) {
        title.to_string()
    } else if let Some(priority @ 0..=3) = directive_integer(&directive, "priority") {
        format!("[P{priority}] {title}")
    } else {
        title.to_string()
    };
    let file_path = Path::new(file);
    let file = file_path
        .strip_prefix(cwd)
        .unwrap_or(file_path)
        .to_string_lossy()
        .replace('\\', "/");
    let location = if start == end {
        format!("{file}:{start}")
    } else {
        format!("{file}:{start}-{end}")
    };

    let (suffix, actions) = strip_line_directives(suffix);
    Some((
        format!("{indent}- {title} — {location}\n{indent}  {body}{suffix}"),
        actions,
    ))
}

pub(crate) fn strip_line_directives(line: &str) -> (String, Vec<GitActionDirective>) {
    let mut visible = String::new();
    let mut actions = Vec::new();
    let mut remaining = line;

    while let Some(start) = remaining.find("::git-") {
        visible.push_str(&remaining[..start]);
        let source = &remaining[start..];
        if let Some(directive) = parse_assistant_directive(source, QuoteEscaping::Literal) {
            if let Some(action) = parse_git_action(&directive) {
                actions.push(action);
            }
            remaining = &source[directive.raw.len()..];
        } else if let Some((_, attributes)) = source.split_once('{')
            && let Some((_, suffix)) = attributes.split_once('}')
        {
            remaining = suffix;
        } else {
            visible.push_str(source);
            return (visible, actions);
        }
    }
    visible.push_str(remaining);
    (visible, actions)
}

fn directive_integer(directive: &AssistantDirective<'_>, name: &str) -> Option<i64> {
    directive
        .attributes
        .get(name)?
        .trim()
        .trim_start_matches(['P', 'p'])
        .parse()
        .ok()
}

fn title_has_priority(title: &str) -> bool {
    let bytes = title.trim_start().as_bytes();
    bytes.len() >= 4
        && bytes[0] == b'['
        && matches!(bytes[1], b'P' | b'p')
        && bytes[2].is_ascii_digit()
        && bytes[3] == b']'
}

fn parse_git_action(directive: &AssistantDirective<'_>) -> Option<GitActionDirective> {
    let cwd = directive.attributes.get("cwd")?.to_string();
    match directive.name {
        "git-stage" => Some(GitActionDirective::Stage { cwd }),
        "git-commit" => Some(GitActionDirective::Commit { cwd }),
        "git-create-branch" => Some(GitActionDirective::CreateBranch {
            cwd,
            branch: directive.attributes.get("branch")?.to_string(),
        }),
        "git-push" => Some(GitActionDirective::Push {
            cwd,
            branch: directive.attributes.get("branch")?.to_string(),
        }),
        "git-create-pr" => Some(GitActionDirective::CreatePr {
            cwd,
            branch: directive.attributes.get("branch")?.to_string(),
            url: directive.attributes.get("url").map(ToString::to_string),
            is_draft: directive
                .attributes
                .get("isDraft")
                .is_some_and(|value| value == "true"),
        }),
        _ => None,
    }
}

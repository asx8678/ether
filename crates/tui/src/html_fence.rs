// New in ether: splits ```` ```view ```` and ```` ```mermaid ```` blocks out of a streamed answer.
//! The answer arrives in arbitrary chunks. [`HtmlFenceFilter::push`] passes text through as
//! soon as it cannot start a drawable fence, holds the fenced body until its closing fence, and
//! then returns it as one [`FenceOutput::Block`], so it can be drawn in place of the code.
//! ```` ```view ```` holds HTML; ```` ```mermaid ```` a Mermaid diagram. ```` ```html ```` stays
//! code, since a coding agent is often asked to show HTML source.

/// What a drawable block holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    /// HTML using the view kit.
    View,
    /// A Mermaid diagram.
    Mermaid,
}

#[derive(Debug, PartialEq)]
pub(crate) enum FenceOutput {
    Text(String),
    /// A complete block: its kind, the body inside the fences, and the fenced source to show if
    /// drawing fails.
    Block {
        kind: BlockKind,
        body: String,
        source: String,
    },
}

#[derive(Debug, Default)]
pub(crate) struct HtmlFenceFilter {
    /// The current line, while it is incomplete.
    line: String,
    block: Option<OpenBlock>,
    /// The start of the current line was already passed on as text, so the line cannot open a
    /// fence (a ```` ```view ```` mentioned mid-sentence stays text).
    mid_line: bool,
    /// Inside another fenced code block (its fence character and length): fences in it are
    /// text, for example a markdown example that shows a ```` ```view ```` block.
    other_fence: Option<(char, usize)>,
}

#[derive(Debug)]
struct OpenBlock {
    kind: BlockKind,
    body: String,
    source: String,
}

impl HtmlFenceFilter {
    pub(crate) fn push(&mut self, delta: &str) -> Vec<FenceOutput> {
        let mut out = Vec::new();
        self.line.push_str(delta);
        while let Some(end) = self.line.find('\n') {
            let line: String = self.line.drain(..=end).collect();
            if std::mem::take(&mut self.mid_line) {
                push_text(&mut out, line);
            } else {
                self.on_line(line, &mut out);
            }
        }
        // Outside a block, an incomplete line is held only while it could still open a fence.
        if self.block.is_none()
            && !self.line.is_empty()
            && (self.mid_line || self.other_fence.is_some() || !could_open(&self.line))
        {
            push_text(&mut out, std::mem::take(&mut self.line));
            self.mid_line = true;
        }
        out
    }

    /// The answer ended: a block whose closing fence is the last, unterminated line is complete;
    /// anything else held comes out as text (an unclosed block stays code).
    pub(crate) fn finish(&mut self) -> Vec<FenceOutput> {
        let mut out = Vec::new();
        let line = std::mem::take(&mut self.line);
        self.mid_line = false;
        self.other_fence = None;
        match self.block.take() {
            Some(mut block) if is_closer(&line) => {
                block.source.push_str(&line);
                out.push(FenceOutput::Block {
                    kind: block.kind,
                    body: block.body,
                    source: block.source,
                });
            }
            Some(block) => push_text(&mut out, block.source + &line),
            None => push_text(&mut out, line),
        }
        out
    }

    fn on_line(&mut self, line: String, out: &mut Vec<FenceOutput>) {
        if let Some((fence_char, length)) = self.other_fence {
            if fence_marker(&line).is_some_and(|(c, n)| c == fence_char && n >= length)
                && line.trim().chars().all(|c| c == fence_char)
            {
                self.other_fence = None;
            }
            push_text(out, line);
            return;
        }
        match self.block.as_mut() {
            None => match opener_kind(&line) {
                Some(kind) => {
                    self.block = Some(OpenBlock {
                        kind,
                        body: String::new(),
                        source: line,
                    });
                }
                None => {
                    // Any other fence starts a code block whose contents stay text.
                    self.other_fence = fence_marker(&line);
                    push_text(out, line);
                }
            },
            Some(block) => {
                block.source.push_str(&line);
                if is_closer(&line) {
                    if let Some(block) = self.block.take() {
                        out.push(FenceOutput::Block {
                            kind: block.kind,
                            body: block.body,
                            source: block.source,
                        });
                    }
                } else {
                    block.body.push_str(&line);
                }
            }
        }
    }
}

fn push_text(out: &mut Vec<FenceOutput>, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(FenceOutput::Text(last)) = out.last_mut() {
        last.push_str(&text);
    } else {
        out.push(FenceOutput::Text(text));
    }
}

/// The fences that open a drawable block.
const FENCES: [(&str, BlockKind); 4] = [
    ("```view", BlockKind::View),
    ("~~~view", BlockKind::View),
    ("```mermaid", BlockKind::Mermaid),
    ("~~~mermaid", BlockKind::Mermaid),
];

/// The kind of block a line like ```` ```view ```` opens (any case, `~~~` too, optional words
/// after a space).
fn opener_kind(line: &str) -> Option<BlockKind> {
    let trimmed = line.trim().to_ascii_lowercase();
    FENCES.iter().find_map(|(fence, kind)| {
        trimmed
            .strip_prefix(fence)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '{']))
            .then_some(*kind)
    })
}

/// The fence character and length a line starts with (```` ``` ```` or `~~~`, three or more).
fn fence_marker(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let first = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let length = trimmed.chars().take_while(|c| *c == first).count();
    (length >= 3).then_some((first, length))
}

fn is_closer(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 3 && (trimmed.chars().all(|c| c == '`') || trimmed.chars().all(|c| c == '~'))
}

/// Whether an incomplete line may still turn out to be a fence opener.
fn could_open(partial: &str) -> bool {
    let trimmed = partial.trim_start().to_ascii_lowercase();
    FENCES
        .iter()
        .any(|(fence, _)| fence.starts_with(&trimmed) || trimmed.starts_with(fence))
}

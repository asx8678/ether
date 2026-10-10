// New in ether: splits ```` ```view ```` blocks out of a streamed answer (HTML views).
//! The answer arrives in arbitrary chunks. [`HtmlFenceFilter::push`] passes text through as
//! soon as it cannot start a view fence, holds the fenced HTML until its closing fence, and
//! then returns it as one [`FenceOutput::Block`], so the view can be rendered in place of the
//! code. Only the `view` tag is a view: ```` ```html ```` stays code, since a coding agent is
//! often asked to show HTML source.

#[derive(Debug, PartialEq)]
pub(crate) enum FenceOutput {
    Text(String),
    /// A complete block: the HTML inside, and the fenced source to show if rendering fails.
    Block { html: String, source: String },
}

#[derive(Debug, Default)]
pub(crate) struct HtmlFenceFilter {
    /// The current line, while it is incomplete.
    line: String,
    block: Option<OpenBlock>,
}

#[derive(Debug)]
struct OpenBlock {
    html: String,
    source: String,
}

impl HtmlFenceFilter {
    pub(crate) fn push(&mut self, delta: &str) -> Vec<FenceOutput> {
        let mut out = Vec::new();
        self.line.push_str(delta);
        while let Some(end) = self.line.find('\n') {
            let line: String = self.line.drain(..=end).collect();
            self.on_line(line, &mut out);
        }
        // Outside a block, an incomplete line is held only while it could still open a fence.
        if self.block.is_none() && !self.line.is_empty() && !could_open(&self.line) {
            push_text(&mut out, std::mem::take(&mut self.line));
        }
        out
    }

    /// The answer ended: whatever is held comes out as text (an unclosed block stays code).
    pub(crate) fn finish(&mut self) -> Vec<FenceOutput> {
        let mut out = Vec::new();
        let mut rest = self
            .block
            .take()
            .map(|block| block.source)
            .unwrap_or_default();
        rest.push_str(&std::mem::take(&mut self.line));
        push_text(&mut out, rest);
        out
    }

    fn on_line(&mut self, line: String, out: &mut Vec<FenceOutput>) {
        match self.block.as_mut() {
            None if is_opener(&line) => {
                self.block = Some(OpenBlock {
                    html: String::new(),
                    source: line,
                });
            }
            None => push_text(out, line),
            Some(block) => {
                block.source.push_str(&line);
                if is_closer(&line) {
                    if let Some(block) = self.block.take() {
                        out.push(FenceOutput::Block {
                            html: block.html,
                            source: block.source,
                        });
                    }
                } else {
                    block.html.push_str(&line);
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

/// The fences that open a view.
const VIEW_FENCES: [&str; 2] = ["```view", "~~~view"];

/// A line like ```` ```view ```` (any case, `~~~` too, optional words after a space).
fn is_opener(line: &str) -> bool {
    let trimmed = line.trim().to_ascii_lowercase();
    VIEW_FENCES.iter().any(|fence| {
        trimmed
            .strip_prefix(fence)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '{']))
    })
}

fn is_closer(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 3 && (trimmed.chars().all(|c| c == '`') || trimmed.chars().all(|c| c == '~'))
}

/// Whether an incomplete line may still turn out to be a view fence opener.
fn could_open(partial: &str) -> bool {
    let trimmed = partial.trim_start().to_ascii_lowercase();
    VIEW_FENCES
        .iter()
        .any(|fence| fence.starts_with(&trimmed) || trimmed.starts_with(fence))
}

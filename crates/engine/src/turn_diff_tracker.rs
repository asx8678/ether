// Modified from OpenAI Codex (codex-rs/core/src/turn_diff_tracker.rs @ 806d973): one local environment (tracked paths carry no environment id, one display root), unit tests removed.
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use sha1::digest::Output;

use codex_apply_patch::AppliedPatchChange;
use codex_apply_patch::AppliedPatchDelta;
use codex_apply_patch::AppliedPatchFileChange;
use codex_utils_path_uri::PathUri;

const ZERO_OID: &str = "0000000000000000000000000000000000000000";
const DEV_NULL: &str = "/dev/null";
const REGULAR_FILE_MODE: &str = "100644";
// Normal edits finish well within 100 ms; pathological inputs fall back to a coarse,
// content-exact diff without stalling tool completion.
const DIFF_TIMEOUT: Duration = Duration::from_millis(100);

struct TrackedContent {
    content: String,
    revision: u64,
}

/// ether has one local environment, so a tracked path is just the path.
type TrackedPath = PathUri;

#[derive(Eq, Hash, PartialEq)]
struct DiffCacheKey {
    left_path: TrackedPath,
    left_revision: Option<u64>,
    right_path: TrackedPath,
    right_revision: Option<u64>,
}

/// Tracks the net text diff for the current turn from committed apply_patch
/// mutations, without rereading the workspace filesystem.
pub struct TurnDiffTracker {
    valid: bool,
    display_root: Option<PathUri>,
    baseline_by_path: HashMap<TrackedPath, TrackedContent>,
    current_by_path: HashMap<TrackedPath, TrackedContent>,
    origin_by_current_path: HashMap<TrackedPath, TrackedPath>,
    next_revision: u64,
    rendered_diffs: HashMap<DiffCacheKey, Option<String>>,
    unified_diff: Option<String>,
}

impl Default for TurnDiffTracker {
    fn default() -> Self {
        Self {
            valid: true,
            display_root: None,
            baseline_by_path: HashMap::new(),
            current_by_path: HashMap::new(),
            origin_by_current_path: HashMap::new(),
            next_revision: 0,
            rendered_diffs: HashMap::new(),
            unified_diff: None,
        }
    }
}

impl TurnDiffTracker {
    /// A tracker that shows paths relative to `display_root` (the session cwd).
    pub(crate) fn with_display_root(display_root: PathUri) -> Self {
        Self {
            display_root: Some(display_root),
            ..Self::default()
        }
    }

    pub(crate) fn track_delta(&mut self, delta: &AppliedPatchDelta) {
        if !self.valid {
            return;
        }

        if !delta.is_exact() {
            self.invalidate();
            return;
        }

        for change in delta.changes() {
            self.apply_change(change);
        }
        self.refresh_unified_diff();
    }

    pub(crate) fn invalidate(&mut self) {
        self.valid = false;
        self.rendered_diffs.clear();
        self.unified_diff = None;
    }

    pub(crate) fn get_unified_diff(&self) -> Option<String> {
        self.unified_diff.clone()
    }

    pub(crate) fn has_unified_diff(&self) -> bool {
        self.unified_diff.is_some()
    }

    fn refresh_unified_diff(&mut self) {
        let rename_pairs = self.rename_pairs();
        let paired_destinations = rename_pairs.values().cloned().collect::<HashSet<_>>();
        let mut handled = HashSet::new();
        let mut paths = self
            .baseline_by_path
            .keys()
            .chain(self.current_by_path.keys())
            .cloned()
            .collect::<Vec<_>>();
        paths.sort_by_key(|path| self.display_path(path));
        paths.dedup();

        let mut previous_diffs = std::mem::take(&mut self.rendered_diffs);
        let mut rendered_diffs = HashMap::new();
        let mut aggregated = String::new();
        for path in paths {
            if !handled.insert(path.clone()) {
                continue;
            }

            if paired_destinations.contains(&path) {
                continue;
            }

            let (left_path, right_path) = if let Some(dest) = rename_pairs.get(&path) {
                handled.insert(dest.clone());
                (&path, dest)
            } else {
                (&path, &path)
            };

            let left_content = self.baseline_by_path.get(left_path);
            let right_content = self.current_by_path.get(right_path);
            let key = DiffCacheKey {
                left_path: left_path.clone(),
                left_revision: left_content.map(|content| content.revision),
                right_path: right_path.clone(),
                right_revision: right_content.map(|content| content.revision),
            };
            let rendered = previous_diffs.remove(&key).unwrap_or_else(|| {
                self.render_diff(
                    left_path,
                    left_content.map(|content| content.content.as_str()),
                    right_path,
                    right_content.map(|content| content.content.as_str()),
                )
            });

            if let Some(diff) = rendered.as_deref() {
                aggregated.push_str(diff);
                if !aggregated.ends_with('\n') {
                    aggregated.push('\n');
                }
            }
            rendered_diffs.insert(key, rendered);
        }

        self.rendered_diffs = rendered_diffs;
        self.unified_diff = (!aggregated.is_empty()).then_some(aggregated);
    }

    fn apply_change(&mut self, change: &AppliedPatchChange) {
        let source_path = change.path.clone();
        match &change.change {
            AppliedPatchFileChange::Add {
                content,
                overwritten_content,
            } => self.apply_add(source_path, content, overwritten_content.as_deref()),
            AppliedPatchFileChange::Delete { content } => self.apply_delete(source_path, content),
            AppliedPatchFileChange::Update {
                move_path,
                old_content,
                overwritten_move_content,
                new_content,
            } => {
                let move_path = move_path.clone();
                self.apply_update(
                    source_path,
                    move_path,
                    old_content,
                    overwritten_move_content.as_deref(),
                    new_content,
                )
            }
        }
    }

    fn apply_add(&mut self, path: TrackedPath, content: &str, overwritten_content: Option<&str>) {
        self.origin_by_current_path.remove(&path);
        if !self.current_by_path.contains_key(&path)
            && !self.baseline_by_path.contains_key(&path)
            && let Some(overwritten_content) = overwritten_content
        {
            let overwritten_content = self.tracked_content(overwritten_content);
            self.baseline_by_path
                .insert(path.clone(), overwritten_content);
        }
        let content = self.tracked_content(content);
        self.current_by_path.insert(path, content);
    }

    fn apply_delete(&mut self, path: TrackedPath, content: &str) {
        if self.current_by_path.remove(&path).is_none()
            && !self.baseline_by_path.contains_key(&path)
        {
            let content = self.tracked_content(content);
            self.baseline_by_path.insert(path.clone(), content);
        }
        self.origin_by_current_path.remove(&path);
    }

    fn apply_update(
        &mut self,
        source_path: TrackedPath,
        move_path: Option<TrackedPath>,
        old_content: &str,
        overwritten_move_content: Option<&str>,
        new_content: &str,
    ) {
        if !self.current_by_path.contains_key(&source_path)
            && !self.baseline_by_path.contains_key(&source_path)
        {
            let old_content = self.tracked_content(old_content);
            self.baseline_by_path
                .insert(source_path.clone(), old_content);
        }

        match move_path {
            Some(dest_path) => {
                if !self.current_by_path.contains_key(&dest_path)
                    && !self.baseline_by_path.contains_key(&dest_path)
                    && let Some(overwritten_move_content) = overwritten_move_content
                {
                    let overwritten_move_content = self.tracked_content(overwritten_move_content);
                    self.baseline_by_path
                        .insert(dest_path.clone(), overwritten_move_content);
                }
                let origin = self
                    .origin_by_current_path
                    .remove(&source_path)
                    .unwrap_or_else(|| source_path.clone());
                self.current_by_path.remove(&source_path);
                let new_content = self.tracked_content(new_content);
                self.current_by_path.insert(dest_path.clone(), new_content);
                self.origin_by_current_path.remove(&dest_path);
                if dest_path != origin {
                    self.origin_by_current_path.insert(dest_path, origin);
                }
            }
            None => {
                let new_content = self.tracked_content(new_content);
                self.current_by_path.insert(source_path, new_content);
            }
        }
    }

    fn tracked_content(&mut self, content: &str) -> TrackedContent {
        let revision = self.next_revision;
        self.next_revision += 1;
        TrackedContent {
            content: content.to_string(),
            revision,
        }
    }

    fn rename_pairs(&self) -> HashMap<TrackedPath, TrackedPath> {
        self.origin_by_current_path
            .iter()
            .filter_map(|(dest_path, origin_path)| {
                if dest_path == origin_path
                    || self.current_by_path.contains_key(origin_path)
                    || !self.current_by_path.contains_key(dest_path)
                    || !self.baseline_by_path.contains_key(origin_path)
                    || self.baseline_by_path.contains_key(dest_path)
                {
                    return None;
                }

                Some((origin_path.clone(), dest_path.clone()))
            })
            .collect()
    }

    fn render_diff(
        &self,
        left_path: &TrackedPath,
        left_content: Option<&str>,
        right_path: &TrackedPath,
        right_content: Option<&str>,
    ) -> Option<String> {
        if left_content == right_content {
            return None;
        }

        // Git diff paths always use `/`, even when the displayed target path is Windows-native.
        let left_display = self.display_path(left_path).replace('\\', "/");
        let right_display = self.display_path(right_path).replace('\\', "/");
        let left_oid = left_content.map_or_else(
            || ZERO_OID.to_string(),
            |content| git_blob_oid(content.as_bytes()),
        );
        let right_oid = right_content.map_or_else(
            || ZERO_OID.to_string(),
            |content| git_blob_oid(content.as_bytes()),
        );

        let mut diff = format!("diff --git a/{left_display} b/{right_display}\n");
        match (left_content, right_content) {
            (None, Some(_)) => diff.push_str(&format!("new file mode {REGULAR_FILE_MODE}\n")),
            (Some(_), None) => diff.push_str(&format!("deleted file mode {REGULAR_FILE_MODE}\n")),
            (Some(_), Some(_)) => {}
            (None, None) => return None,
        }

        diff.push_str(&format!("index {left_oid}..{right_oid}\n"));

        let old_header = if left_content.is_some() {
            format!("a/{left_display}")
        } else {
            DEV_NULL.to_string()
        };
        let new_header = if right_content.is_some() {
            format!("b/{right_display}")
        } else {
            DEV_NULL.to_string()
        };

        let mut config = similar::TextDiff::configure();
        config.timeout(DIFF_TIMEOUT);
        let unified = config
            .diff_lines(left_content.unwrap_or(""), right_content.unwrap_or(""))
            .unified_diff()
            .context_radius(3)
            .header(&old_header, &new_header)
            .to_string();
        diff.push_str(&unified);
        Some(diff)
    }

    fn display_path(&self, path: &TrackedPath) -> String {
        self.display_root
            .as_ref()
            .and_then(|root| path.relative_path_from(root))
            .unwrap_or_else(|| path.inferred_native_path_string())
    }
}

fn git_blob_oid(data: &[u8]) -> String {
    format!("{:x}", git_blob_sha1_hex_bytes(data))
}

/// Compute the Git SHA-1 blob object ID for the given content (bytes).
fn git_blob_sha1_hex_bytes(data: &[u8]) -> Output<sha1::Sha1> {
    let header = format!("blob {}\0", data.len());
    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(data);
    hasher.finalize()
}

/// The session's diff for the running turn, in the role of Codex's turn-scoped
/// `SharedTurnDiffTracker`. It is keyed by turn id, so a new turn starts a fresh tracker without
/// a reset hook.
#[derive(Default)]
pub(crate) struct TurnDiffs {
    turn_id: String,
    tracker: TurnDiffTracker,
    /// A patch changed the tracker since the last `TurnDiff` event.
    changed: bool,
    /// A non-empty diff was reported this turn, so an invalidation must be reported too.
    reported: bool,
}

impl TurnDiffs {
    fn for_turn(&mut self, turn_id: &str, display_root: &PathUri) -> &mut Self {
        if self.turn_id != turn_id {
            *self = Self {
                turn_id: turn_id.to_string(),
                tracker: TurnDiffTracker::with_display_root(display_root.clone()),
                changed: false,
                reported: false,
            };
        }
        self
    }

    /// Records what a patch committed. `None` means the committed changes are unknown, which
    /// invalidates the turn diff, as Codex does for a failed patch without a delta.
    pub(crate) fn record(
        &mut self,
        turn_id: &str,
        display_root: &PathUri,
        delta: Option<&AppliedPatchDelta>,
    ) {
        let this = self.for_turn(turn_id, display_root);
        match delta {
            Some(delta) if delta.is_empty() => return,
            Some(delta) => this.tracker.track_delta(delta),
            None => this.tracker.invalidate(),
        }
        this.changed = true;
    }

    /// The diff to report in a `TurnDiff` event, if a patch changed it since the last report.
    pub(crate) fn take_changed(&mut self, turn_id: &str) -> Option<String> {
        if self.turn_id != turn_id || !self.changed {
            return None;
        }
        self.changed = false;
        let had_diff = self.reported;
        let diff = self.tracker.get_unified_diff();
        self.reported = diff.is_some();
        (had_diff || self.tracker.has_unified_diff()).then(|| diff.unwrap_or_default())
    }
}

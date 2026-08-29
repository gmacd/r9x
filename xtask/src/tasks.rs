//! `xtask tasks` — validate the `tasks/` tree (task 129).
//!
//! The tree is one file per task: open and parked tasks at the top level,
//! finished ones in `done/`, design docs in `plans/` (not tasks), and
//! `todo.md` as the hand-annotated index. Each task file carries front
//! matter (`id`, `status`, `wave`, `depends-on`, `commit`, `issue`) and, in
//! the current era, a prose `## Status:` line. Nothing here trusts either
//! side without checking the other.
//!
//! Reporting discipline: **report, do not block.** Findings print to stdout
//! and the process exits zero, so CI gets a short honest list beside
//! `fmt --check`. A non-zero exit is reserved for malformed front matter,
//! which is a defect in a task file rather than a drift signal.
//!
//! Prose references (`task 119`, `Depends on 118, 121`) resolve against the
//! known ids: every front matter `id:` plus every number in `todo.md`'s
//! list (which is the identity for slug-named tasks that predate `id:`).
//! Files under `done/` are exempt from reference resolution — they are the
//! historical record, and the pre-migration numbering they cite (tasks 39,
//! 70, 86, the arc ordinals) was never carried into the tree.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::result::Result;

/// The only keys a task file's front matter may carry.
const ALLOWED_KEYS: [&str; 6] = ["id", "status", "wave", "depends-on", "commit", "issue"];

/// The index files: not tasks, so exempt from every file-level check.
const INDEX_FILES: [&str; 3] = ["README.md", "todo.md", "done.md"];

/// `todo.md`'s generated region. `--fix` only ever writes between these
/// markers; the narrative above the begin marker (the dated audit log) and
/// the hand-written section intros and per-entry notes inside the region
/// are preserved verbatim.
const BEGIN_MARKER: &str = "<!-- xtask:tasks begin -->";
const END_MARKER: &str = "<!-- xtask:tasks end -->";

/// A finding. `Malformed` is the only class that fails the run.
#[derive(Debug, PartialEq, Eq)]
enum Finding {
    Malformed { file: String, detail: String },
    StatusMismatch { file: String, front: String, prose: String },
    Misplaced { file: String, detail: String },
    UnresolvedRef { file: String, line: u32, id: String },
    DuplicateId { id: String, files: Vec<String> },
    MissingCommit { file: String, candidate: String },
    IndexDrift { detail: String },
}

impl Finding {
    fn render(&self) -> String {
        match self {
            Finding::Malformed { file, detail } => format!("malformed      {file}: {detail}"),
            Finding::StatusMismatch { file, front, prose } => {
                format!("status/prose   {file}: front matter says `{front}`, prose says `{prose}`")
            }
            Finding::Misplaced { file, detail } => format!("misplaced      {file}: {detail}"),
            Finding::UnresolvedRef { file, line, id } => {
                // line 0 marks a front matter reference (no line of its own).
                let loc = if *line == 0 {
                    format!("{file} (front matter)")
                } else {
                    format!("{file}:{line}")
                };
                format!("unresolved-ref {loc}: 'task {id}' names no known task (id or list number)")
            }
            Finding::DuplicateId { id, files } => {
                format!("duplicate-id   {id}: claimed by {}", files.join(", "))
            }
            Finding::MissingCommit { file, candidate } => {
                format!(
                    "missing-commit {file}: no `commit:` field, recoverable from its Status line ({candidate})"
                )
            }
            Finding::IndexDrift { detail } => format!("index-drift    {detail}"),
        }
    }
}

/// One task file, front matter parsed and value-checked.
struct TaskFile {
    /// Path relative to `tasks/`: `foo.md` or `done/foo.md`.
    rel: String,
    in_done_dir: bool,
    /// `id:` if present; slug-named tasks key by filename instead.
    id: Option<String>,
    /// `status:` — required; empty when the block was unreadable or the
    /// file has no front matter at all (both reported as malformed).
    status: String,
    /// `depends-on:` items, format-checked at parse time.
    depends_on: Vec<String>,
    /// `commit:` if present.
    commit: Option<String>,
    /// Format errors in the front matter, if any.
    malformed: Vec<String>,
    /// Header line count (see FrontMatter): body line numbers + this + 2
    /// are file line numbers.
    header_lines: u32,
    /// The first prose `## Status:` line's word, if the file has one.
    /// Older tasks predate the convention and carry no such line.
    prose_status: Option<String>,
    /// The `## Status:` line verbatim (for commit recovery).
    status_line: Option<String>,
    /// Body text (after the closing `---`), for reference extraction.
    body: String,
}

struct Tree {
    tasks: Vec<TaskFile>,
    /// id → files claiming it (front matter ids; list numbers are checked
    /// against this in the index checks).
    id_claims: BTreeMap<String, BTreeSet<String>>,
    /// Every resolvable id: front matter ids plus todo.md list numbers.
    known_ids: BTreeSet<String>,
    /// todo.md list entries: (number, linked filename, 1-based file line).
    listed: Vec<(String, String, u32)>,
    /// Whether todo.md carries the generated-region markers; without them
    /// the list is not reconcilable and the per-entry drift checks would
    /// just report its absence once per task.
    todo_marked: bool,
}

/// `cargo xtask tasks --check` (default) or `--fix`, over the repo's
/// `tasks/` directory.
///
/// Ok means the run completed; findings are printed, not errors. Err (and
/// hence a non-zero exit) is reserved for malformed front matter.
pub fn run(with_fix: bool) -> Result<(), Box<dyn std::error::Error>> {
    run_on(&crate::workspace().join("tasks"), with_fix)
}

/// The check/fix proper, over an explicit directory (the tests point it at
/// fixtures).
pub fn run_on(tasks_dir: &Path, with_fix: bool) -> Result<(), Box<dyn std::error::Error>> {
    if with_fix {
        return fix(tasks_dir);
    }

    let tree = load(tasks_dir)?;
    let mut findings: Vec<Finding> = Vec::new();
    findings.extend(file_findings(&tree));
    findings.extend(index_findings(&tree));

    let malformed = findings.iter().filter(|f| matches!(f, Finding::Malformed { .. })).count();
    if findings.is_empty() {
        println!("xtask tasks: tasks/ is consistent ({} files checked)", tree.tasks.len());
    } else {
        println!("xtask tasks: {} finding(s) in tasks/", findings.len());
        for f in &findings {
            println!("  {}", f.render());
        }
    }

    if malformed > 0 {
        return Err(format!("{malformed} malformed front matter block(s)").into());
    }
    Ok(())
}

/// Read and parse every task file. Err on unreadable input or a missing
/// front matter block (structural malformation); value-level malformation
/// becomes `Finding::Malformed` so the full list still prints.
fn load(tasks_dir: &Path) -> Result<Tree, Box<dyn std::error::Error>> {
    let mut tasks: Vec<TaskFile> = Vec::new();

    let scan = |dir: &Path, in_done_dir: bool, out: &mut Vec<TaskFile>| -> std::io::Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if INDEX_FILES.contains(&name.as_str()) && !in_done_dir {
                continue;
            }
            let rel = if in_done_dir { format!("done/{name}") } else { name.clone() };
            let text = match fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    out.push(TaskFile::malformed_only(
                        rel,
                        in_done_dir,
                        format!("cannot read: {e}"),
                    ));
                    continue;
                }
            };
            let fm = match parse_front_matter(&text) {
                Some(fm) => fm,
                None => {
                    out.push(TaskFile::malformed_only(
                        rel,
                        in_done_dir,
                        "no front matter block".into(),
                    ));
                    continue;
                }
            };
            out.push(fm.into_task_file(rel, in_done_dir));
        }
        Ok(())
    };

    scan(tasks_dir, false, &mut tasks)?;
    scan(&tasks_dir.join("done"), true, &mut tasks)?;

    let mut id_claims: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut known_ids: BTreeSet<String> = BTreeSet::new();
    for t in &tasks {
        if let Some(id) = &t.id {
            id_claims.entry(id.clone()).or_default().insert(t.rel.clone());
            known_ids.insert(id.clone());
        }
    }

    let todo_text = fs::read_to_string(tasks_dir.join("todo.md")).unwrap_or_default();
    let listed = parse_todo_list(&todo_text);
    for (num, _, _) in &listed {
        known_ids.insert(num.clone());
    }

    Ok(Tree { tasks, id_claims, known_ids, listed, todo_marked: todo_region(&todo_text).is_some() })
}

impl TaskFile {
    /// A file that could not even be parsed: every field empty, one
    /// malformed error, and all semantic checks skipped.
    fn malformed_only(rel: String, in_done_dir: bool, detail: String) -> Self {
        Self {
            rel,
            in_done_dir,
            id: None,
            status: String::new(),
            depends_on: Vec::new(),
            commit: None,
            malformed: vec![detail],
            header_lines: 0,
            prose_status: None,
            status_line: None,
            body: String::new(),
        }
    }
}

/// The parsed front matter block, or None when the file does not start
/// with one. Value formats are checked here so a malformed value is a
/// parse failure, not a silently wrong reference.
struct FrontMatter {
    id: Option<String>,
    status: Option<String>,
    depends_on: Vec<String>,
    commit: Option<String>,
    body: String,
    /// Lines of the header (block lines, excluding the `---` fences): the
    /// offset that turns a body line number into a file line number.
    header_lines: u32,
    errors: Vec<String>,
}

fn parse_front_matter(text: &str) -> Option<FrontMatter> {
    // Walk the raw text, keeping each line's terminator: `str::lines()`
    // would strip a CRLF's `\r` and skew the body offset. The body is
    // sliced from this scan rather than found by a second search, so a
    // closing fence with trailing whitespace can never void the
    // body-level checks.
    let mut line_start = 0;
    let mut block: Vec<&str> = Vec::new();
    let mut close_end: Option<usize> = None;
    for (i, raw) in text.split_inclusive('\n').enumerate() {
        let line = raw.trim_end_matches('\n');
        if line.trim() == "---" {
            if i == 0 {
                line_start += raw.len();
            } else {
                close_end = Some(line_start + raw.len());
                break;
            }
        } else {
            if i > 0 {
                block.push(line);
            }
            line_start += raw.len();
        }
    }
    let close_end = close_end?;

    let mut errors = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut id = None;
    let mut status = None;
    let mut depends_on: Vec<String> = Vec::new();
    let mut commit = None;

    for line in &block {
        let (key, value) = match line.split_once(':') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => {
                errors.push(format!("front matter line has no `key: value` separator: {line:?}"));
                continue;
            }
        };
        if !seen.insert(key) {
            errors.push(format!("duplicate front matter key `{key}`"));
        }
        if !ALLOWED_KEYS.contains(&key) {
            errors.push(format!("unknown front matter key `{key}`"));
            continue;
        }
        match key {
            "id" => {
                if !is_task_id(value) {
                    errors.push(format!(
                        "`id: {value}` is not a task id (digits, optionally one letter)"
                    ));
                }
                id = Some(value.to_string());
            }
            "status" => {
                if !matches!(value, "open" | "parked" | "done") {
                    errors.push(format!("`status: {value}` is not open, parked or done"));
                }
                status = Some(value.to_string());
            }
            "wave" | "issue" if !is_decimal(value) => {
                errors.push(format!("`{key}: {value}` is not a number"));
            }
            "depends-on" => {
                for item in value.split(',') {
                    let item = item.trim();
                    if !is_task_id(item) {
                        errors.push(format!("`depends-on` item `{item}` is not a task id"));
                    }
                    depends_on.push(item.to_string());
                }
            }
            "commit" => {
                if !is_commit_hash(value) {
                    errors.push(format!("`commit: {value}` is not a 7–40 character hex hash"));
                }
                commit = Some(value.to_string());
            }
            _ => {}
        }
    }

    if status.is_none() {
        errors.push("front matter has no `status:`".to_string());
    }

    let body = text[close_end..].to_string();

    Some(FrontMatter {
        id,
        status,
        depends_on,
        commit,
        body,
        header_lines: block.len() as u32,
        errors,
    })
}

impl FrontMatter {
    fn into_task_file(self, rel: String, in_done_dir: bool) -> TaskFile {
        // A malformed block's values are dropped so no semantic check
        // acts on them; the errors themselves are reported.
        let malformed = !self.errors.is_empty();
        let (id, status, depends_on, commit) = if malformed {
            (None, String::new(), Vec::new(), None)
        } else {
            (self.id, self.status.unwrap_or_default(), self.depends_on, self.commit)
        };

        let mut prose_status = None;
        let mut status_line = None;
        for line in self.body.lines() {
            if let Some(rest) = line.strip_prefix("## Status:") {
                status_line = Some(line.to_string());
                prose_status = Some(rest.split_whitespace().next().unwrap_or("").to_string());
                break;
            }
        }

        TaskFile {
            rel,
            in_done_dir,
            id,
            status,
            depends_on,
            commit,
            malformed: self.errors,
            header_lines: self.header_lines,
            prose_status,
            status_line,
            body: self.body,
        }
    }
}

/// Per-file checks: malformed blocks, status/prose agreement, placement,
/// prose references, recoverable commits.
fn file_findings(tree: &Tree) -> Vec<Finding> {
    let mut findings = Vec::new();
    for t in &tree.tasks {
        for detail in &t.malformed {
            findings.push(Finding::Malformed { file: t.rel.clone(), detail: detail.clone() });
        }
        // Malformed blocks have their values dropped; the semantic checks
        // below would act on empties.
        if !t.malformed.is_empty() || t.status.is_empty() {
            continue;
        }

        // Status versus prose. A missing `## Status:` line is the older
        // convention, not a disagreement.
        if let Some(prose) = &t.prose_status
            && (prose != &t.status || !matches!(prose.as_str(), "open" | "parked" | "done"))
        {
            findings.push(Finding::StatusMismatch {
                file: t.rel.clone(),
                front: t.status.clone(),
                prose: prose.clone(),
            });
        }

        // Placement: done files live in done/, and nothing else does.
        if t.in_done_dir && t.status != "done" {
            findings.push(Finding::Misplaced {
                file: t.rel.clone(),
                detail: format!("status is `{}`, but the file is in done/", t.status),
            });
        }
        if !t.in_done_dir && t.status == "done" {
            findings.push(Finding::Misplaced {
                file: t.rel.clone(),
                detail: "status is `done`, but the file is at the top level (move it to done/)"
                    .to_string(),
            });
        }

        // Prose references. Files under done/ are the historical record
        // and cite pre-migration ids that were never carried into the
        // tree; policing them would trade one drift for forty false
        // positives. The live tree is what must point at real tasks.
        if !t.in_done_dir {
            for (body_line, id) in extract_refs(&t.body) {
                if !tree.known_ids.contains(id) {
                    findings.push(Finding::UnresolvedRef {
                        file: t.rel.clone(),
                        // Body line numbers start after the header: two
                        // fences plus the block's lines.
                        line: t.header_lines + 2 + body_line,
                        id: id.to_string(),
                    });
                }
            }
            for id in &t.depends_on {
                if !tree.known_ids.contains(id) {
                    findings.push(Finding::UnresolvedRef {
                        file: t.rel.clone(),
                        // line 0 marks a front matter reference (no line of its own).
                        line: 0,
                        id: id.clone(),
                    });
                }
            }
        }

        // A done task whose Status line carries a hash but whose front
        // matter lost it. The hash is recoverable because the Status line
        // is the file's own landing record; a hash elsewhere in the prose
        // is not (it may be the commit under review, a related fix, an
        // address).
        if t.status == "done"
            && t.commit.is_none()
            && let Some(hash) = recover_commit_from_status_line(&t.status_line)
        {
            findings.push(Finding::MissingCommit { file: t.rel.clone(), candidate: hash });
        }
    }
    findings
}

/// The first hash inside the first parenthesised group of a
/// `## Status: done (...)` line, if any.
fn recover_commit_from_status_line(status_line: &Option<String>) -> Option<String> {
    let line = status_line.as_ref()?;
    let rest = line.strip_prefix("## Status:")?;
    let parens = rest.split('(').nth(1)?.split(')').next()?;
    parens.split_whitespace().find_map(|tok| {
        let tok = tok.trim_matches(|c| c == ',' || c == ')' || c == '(');
        is_commit_hash(tok).then_some(tok.to_string())
    })
}

/// Index checks: duplicate ids (front matter ids and list numbers are both
/// identities) and the list agreeing with the tree in both directions.
fn index_findings(tree: &Tree) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Marker-independent: front matter ids are identities regardless of
    // whether the list region is present.
    for (id, files) in &tree.id_claims {
        if files.len() > 1 {
            findings.push(Finding::DuplicateId {
                id: id.clone(),
                files: files.iter().cloned().collect(),
            });
        }
    }

    if !tree.todo_marked {
        findings.push(Finding::IndexDrift {
            detail:
                "todo.md carries no generated-region markers; its list is not reconcilable by --fix"
                    .into(),
        });
    } else {
        // List numbers are identities too: two entries sharing a number
        // collide, and a number claimed by a file's id while the entry links
        // a different file is a duplicate as well.
        let mut nums: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
        for (num, file, line) in &tree.listed {
            nums.entry(num.as_str()).or_default().push(*line);
            // Compare by basename: a task that moved to done/ keeps its
            // bare filename in a lingering entry while claimants carry
            // the moved rel path (`done/foo.md`).
            let file_base = file.rsplit('/').next().unwrap_or(file);
            if let Some(claimants) = tree.id_claims.get(num.as_str())
                && !claimants.iter().any(|c| c.rsplit('/').next() == Some(file_base))
            {
                // Both claimants belong in the message: the list entry and the
                // file whose front matter carries the same id.
                let mut files = vec![format!("todo.md:{line}")];
                files.extend(claimants.iter().cloned());
                findings.push(Finding::DuplicateId { id: num.clone(), files });
            }
        }
        for (num, lines) in &nums {
            if lines.len() > 1 {
                findings.push(Finding::DuplicateId {
                    id: num.to_string(),
                    files: lines.iter().map(|l| format!("todo.md:{l}")).collect(),
                });
            }
        }

        // Every open and parked task must be listed; done ones must not be.
        let listed_files: BTreeSet<&str> = tree.listed.iter().map(|(_, f, _)| f.as_str()).collect();
        for t in &tree.tasks {
            if t.in_done_dir || t.status.is_empty() || t.status == "done" {
                continue;
            }
            let name = t.rel.rsplit('/').next().unwrap_or(&t.rel);
            if !listed_files.contains(name) {
                findings.push(Finding::IndexDrift {
                    detail: format!("{name} is {} but not listed in todo.md", t.status),
                });
            }
        }
        for (num, file, line) in &tree.listed {
            let task = tree.tasks.iter().find(|t| t.rel.rsplit('/').next() == Some(file.as_str()));
            match task {
                None => findings.push(Finding::IndexDrift {
                    detail: format!(
                        "todo.md:{line} lists {file}, which is not in the tree (number {num})"
                    ),
                }),
                Some(t) if t.in_done_dir || t.status == "done" => {
                    findings.push(Finding::IndexDrift {
                        detail: format!(
                            "todo.md:{line} lists {file} (number {num}), which is done"
                        ),
                    })
                }
                _ => {}
            }
        }
    }
    findings
}

/// `todo.md`'s list entries inside the generated region:
/// `N. [file](file) ...`. The number is the task's identity for
/// slug-named tasks; the linked filename is the check key. Lines outside
/// the markers (the narrative) may mention the same shape and are ignored.
fn parse_todo_list(text: &str) -> Vec<(String, String, u32)> {
    let Some((begin, end)) = todo_region(text) else {
        return Vec::new();
    };
    // 1-based file line of the region's first line.
    let region_start = begin + BEGIN_MARKER.len();
    let line_at_region_start = text[..region_start].bytes().filter(|&b| b == b'\n').count() + 1;
    let region = &text[region_start..end];

    let mut out = Vec::new();
    for (i, line) in region.lines().enumerate() {
        let (number, file) = match parse_entry_head(line) {
            Some(h) => h,
            None => continue,
        };
        out.push((number.to_string(), file.to_string(), (line_at_region_start + i) as u32));
    }
    out
}

/// `todo.md`'s generated region: the begin marker's start and the end
/// marker's start, when both are present in order.
fn todo_region(text: &str) -> Option<(usize, usize)> {
    match (text.find(BEGIN_MARKER), text.find(END_MARKER)) {
        (Some(b), Some(e)) if e > b => Some((b, e)),
        _ => None,
    }
}

/// `N. [file](file) ...` on an entry line; None otherwise. The link text
/// and target must be identical — list entries link the task file to
/// itself, and the equality keeps prose that happens to carry a numbered
/// markdown link from being read as an entry.
fn parse_entry_head(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let (number, rest) = trimmed.split_once(". [")?;
    if !is_task_id(number) {
        return None;
    }
    let (file, rest) = rest.split_once(']')?;
    let target = rest.strip_prefix('(')?.split(')').next()?;
    if file.is_empty() || target != file {
        return None;
    }
    Some((number, file))
}

/// Extract task references from prose. Returns (1-based line number, id).
///
/// Recognised phrasings: `task 119`, `Task 88`, `task-88`, `tasks 106`
/// (the id, not the range, when the prose writes `102–108`), and
/// `Depends on 118, 121`. Ordinals of the form `Task 4 of 7` (the
/// gates-hardening arc numbers its members this way) are skipped: the
/// number is a position in a plan, not a task.
fn extract_refs(body: &str) -> Vec<(u32, &str)> {
    let mut out = Vec::new();
    for (i, line) in body.lines().enumerate() {
        let line_no = (i + 1) as u32;
        let lower = line.to_ascii_lowercase();
        let bytes = line.as_bytes();
        let mut pos = 0;
        loop {
            let rest = &lower[pos..];
            let Some(rel) = rest.find("task") else { break };
            let start = pos + rel;
            // Word boundary: `task` must not be the tail of a longer word.
            if start > 0 && bytes[start - 1].is_ascii_alphabetic() {
                pos = start + 1;
                continue;
            }
            let is_plural = lower[start..].starts_with("tasks");
            let after = start + "task".len() + usize::from(is_plural);
            let Some(&sep) = bytes.get(after) else { break };
            let sep_len = match sep {
                b' ' | b'-' => 1,
                _ => {
                    pos = start + 1;
                    continue;
                }
            };
            let num_start = after + sep_len;
            let digits = line[num_start..].bytes().take_while(|c| c.is_ascii_digit()).count();
            let num_end = num_start + digits;
            let mut id_end = num_end;
            // Any alphabetic char after the digits: the id shape is
            // lowercase (`is_task_id`), so an uppercase suffix (`Task 74B`)
            // is consumed and reported as `74B`, not silently read as 74.
            if num_end < line.len() && line.as_bytes()[num_end].is_ascii_alphabetic() {
                id_end = num_end + 1;
            }
            if digits > 0 {
                let id = &line[num_start..id_end];
                // `Task 4 of 7`: ordinal in an arc (the number is a
                // position in a plan), not a task id. The `of` must be
                // followed by a number — `task 118 of the arc` is a real
                // reference. Compared lowercased: `Task 4 Of 7` is an
                // ordinal too.
                let ordinal = line[id_end..]
                    .trim_start()
                    .to_ascii_lowercase()
                    .strip_prefix("of ")
                    .is_some_and(|m| m.starts_with(|c: char| c.is_ascii_digit()));
                if !ordinal {
                    out.push((line_no, id));
                }
            }
            // Every path above advances `pos` past this match; the max
            // keeps the empty-id case (separator, no digits) moving too.
            pos = id_end.max(start + 1);
        }
        // `Depends on 118, 121` — the status-line dependency phrasing.
        // `lower` is used for the search; the run it delimits is digits,
        // commas and spaces, which `to_ascii_lowercase` leaves untouched.
        let mut search_from = 0;
        while let Some(rel) = lower[search_from..].find("depends on") {
            let mut j = search_from + rel + "depends on".len();
            while j < line.len() && line.as_bytes()[j].is_ascii_whitespace() {
                j += 1;
            }
            // The run is digits, commas and spaces, plus one letter
            // immediately after a digit (lettered ids: `Depends on 74b, 80`).
            let mut run_end = j;
            while run_end < line.len() {
                let c = line.as_bytes()[run_end];
                let after_digit = run_end > j && line.as_bytes()[run_end - 1].is_ascii_digit();
                if c.is_ascii_digit()
                    || c == b','
                    || c.is_ascii_whitespace()
                    || (c.is_ascii_lowercase() && after_digit)
                {
                    run_end += 1;
                } else {
                    break;
                }
            }
            for part in line[j..run_end].split(',') {
                let part = part.trim();
                if is_task_id(part) {
                    out.push((line_no, part));
                }
            }
            search_from = run_end;
        }
    }
    out
}

/// A task id: one or more digits, optionally one letter as a trailing
/// suffix (task 74b) — never mid-run (`12a3` is not an id).
fn is_task_id(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() {
        return false;
    }
    let digits = if bytes.len() > 1 && bytes[bytes.len() - 1].is_ascii_lowercase() {
        &s[..s.len() - 1]
    } else {
        s
    };
    digits.bytes().all(|c| c.is_ascii_digit())
}

/// A task that must appear in todo.md's list: live (not in done/) and
/// not finished.
fn is_live_open(t: &TaskFile) -> bool {
    !t.in_done_dir && (t.status == "open" || t.status == "parked")
}

/// The live task a filename names, if one exists. Lookup is by basename
/// (a slug-named task's identity); liveness is part of the lookup, so
/// the answer does not depend on which of a same-named `x.md` and
/// `done/x.md` `load` scanned first.
fn live_task_named<'a>(tasks: &'a [TaskFile], name: &str) -> Option<&'a TaskFile> {
    tasks.iter().find(|t| t.rel.rsplit('/').next() == Some(name) && is_live_open(t))
}

/// A git hash: 7–40 hex characters. All-digit hashes exist (4144216), so
/// no letter is required; a decimal run that is actually an address can
/// only reach the Status-line recovery path, whose candidate is advisory
/// — a misread is a note to a human, not a write.
fn is_commit_hash(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// A positive decimal: wave numbers and issue numbers.
fn is_decimal(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// `cargo xtask tasks --fix`: reconcile the list entries inside
/// `todo.md`'s generated region.
///
/// Entry blocks whose task is done or gone are dropped; open tasks with an
/// id that are not listed get a bare entry appended at the end of the
/// region (a human places it into a section — the sections and the
/// per-entry notes are hand-written and never regenerated). The narrative
/// above the begin marker is preserved byte for byte.
///
/// The writer's trust base is the parse: a malformed file's dropped values
/// make it look done, so reconciling against it would delete the file's
/// entry and its hand-written note. Malformed front matter therefore stops
/// the run before anything is written, with the same exit as `--check`.
fn fix(tasks_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let tree = load(tasks_dir)?;

    let findings = file_findings(&tree);
    let malformed = findings.iter().filter(|f| matches!(f, Finding::Malformed { .. })).count();
    if malformed > 0 {
        println!("xtask tasks: {malformed} malformed front matter block(s); not touching todo.md");
        for f in &findings {
            println!("  {}", f.render());
        }
        return Err(format!("{malformed} malformed front matter block(s)").into());
    }

    if !findings.is_empty() {
        println!("xtask tasks: {} finding(s) in tasks/", findings.len());
        for f in &findings {
            println!("  {}", f.render());
        }
    }

    let todo_path = tasks_dir.join("todo.md");
    let text =
        fs::read_to_string(&todo_path).map_err(|e| format!("read {}: {e}", todo_path.display()))?;
    let Some((begin, end)) = todo_region(&text) else {
        return Err(
            "todo.md carries no generated-region markers (add `<!-- xtask:tasks begin -->` / `<!-- xtask:tasks end -->`)".into(),
        );
    };

    let narrative = &text[..begin + BEGIN_MARKER.len()];
    let region = &text[begin + BEGIN_MARKER.len()..end];
    // `end` is the marker's start; the tail starts after it, or every
    // write would append a fresh marker on top of the old one.
    let tail = &text[end + END_MARKER.len()..];

    let open: BTreeSet<&str> = tree
        .tasks
        .iter()
        .filter(|t| is_live_open(t))
        .map(|t| t.rel.rsplit('/').next().unwrap_or(&t.rel))
        .collect();

    let lines: Vec<&str> = region.lines().collect();
    let mut rebuilt: Vec<String> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    let mut kept: BTreeSet<&str> = BTreeSet::new();

    // An entry block runs from its `N. [file](file)` line to the next
    // entry or any header line (a `#` in column 0 ends a block, whatever
    // its level, so a `### ` sub-header is never swallowed into the block
    // above it); everything else is hand-written and kept. Blank lines
    // after an entry belong to the block, so a dropped entry takes its
    // own separator with it.
    let mut i = 0;
    while i < lines.len() {
        // Bind the line in its own statement: `i` is read here and written
        // in the else branch, and the workspace's mixed-read-write lint
        // does not sequence a let-else.
        let line = lines[i];
        let Some((number, file)) = parse_entry_head(line) else {
            rebuilt.push(line.to_string());
            i += 1;
            continue;
        };
        let block_start = i;
        // The block's end is the next entry or header line — start
        // looking after the entry line itself, or the loop below never
        // advances.
        let mut j = i + 1;
        while j < lines.len() && parse_entry_head(lines[j]).is_none() && !lines[j].starts_with('#')
        {
            j += 1;
        }
        let block = &lines[block_start..j];
        i = j;
        let name = file.rsplit('/').next().unwrap_or(file);
        if live_task_named(&tree.tasks, name).is_some() {
            rebuilt.extend(block.iter().map(|l| l.to_string()));
            kept.insert(name);
        } else {
            dropped.push(format!("{number}. {file}"));
        }
    }

    let mut added: Vec<String> = Vec::new();
    for name in &open {
        if kept.contains(name) {
            continue;
        }
        let t = live_task_named(&tree.tasks, name).expect("filtered from tree.tasks above");
        match &t.id {
            Some(id) => {
                // The H1, minus its `# Task N:` prefix — the entry's
                // number already carries the id.
                let title = t
                    .body
                    .lines()
                    .find(|l| l.starts_with("# "))
                    .map(|l| {
                        let t = l.trim_start_matches("# ");
                        t.strip_prefix(&format!("Task {id}: ")).unwrap_or(t).to_string()
                    })
                    .unwrap_or_else(|| "untitled".to_string());
                rebuilt.push(String::new());
                rebuilt.push(format!("{id}. [{name}]({name}) — {title}"));
                added.push(format!("{id}. {name} (unplaced — move it into a section)"));
            }
            None => added.push(format!("{name} (no id — assign one, then re-run)")),
        }
    }

    if dropped.is_empty() && added.is_empty() {
        println!("xtask tasks: todo.md's list already matches the tree");
        return Ok(());
    }

    let mut new_region = rebuilt.join("\n");
    // The markers sit on their own lines, so the region ends with a
    // newline; keep that so the end marker never fuses with the last line.
    if region.ends_with('\n') {
        new_region.push('\n');
    }
    let new_text = format!("{narrative}{new_region}{END_MARKER}{tail}");
    fs::write(&todo_path, new_text)?;

    println!("xtask tasks: regenerated todo.md's list region");
    for d in &dropped {
        println!("  dropped: {d}");
    }
    for a in &added {
        println!("  added:   {a}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fm(text: &str) -> FrontMatter {
        parse_front_matter(text).expect("test text has a front matter block")
    }

    #[test]
    fn front_matter_well_formed() {
        let f = fm(
            "---\nid: 129\nstatus: open\nwave: 1\ndepends-on: 118, 121\ncommit: d773a37\nissue: 47\n---\n\n# body\n",
        );
        assert!(f.errors.is_empty(), "{:?}", f.errors);
        assert_eq!(f.id.as_deref(), Some("129"));
        assert_eq!(f.status.as_deref(), Some("open"));
        assert_eq!(f.depends_on, vec!["118", "121"]);
        assert_eq!(f.commit.as_deref(), Some("d773a37"));
        // The body keeps the document's blank line after the fence; it is
        // what makes body line numbers + header_lines + 2 file-true.
        assert_eq!(f.body, "\n# body\n");
        assert_eq!(f.header_lines, 6);
    }

    #[test]
    fn front_matter_omits_optional_keys() {
        let f = fm("---\nstatus: parked\n---\n");
        assert!(f.errors.is_empty(), "{:?}", f.errors);
        assert_eq!(f.id, None);
        assert!(f.depends_on.is_empty());
    }

    #[test]
    fn front_matter_rejects_unknown_key() {
        let f = fm("---\nstatus: open\npriority: high\n---\n");
        assert!(f.errors.iter().any(|e| e.contains("unknown front matter key `priority`")));
    }

    #[test]
    fn front_matter_rejects_duplicate_key() {
        let f = fm("---\nstatus: open\nstatus: done\n---\n");
        assert!(f.errors.iter().any(|e| e.contains("duplicate front matter key `status`")));
    }

    #[test]
    fn front_matter_requires_status() {
        let f = fm("---\nid: 1\n---\n");
        assert!(f.errors.iter().any(|e| e.contains("no `status:`")));
    }

    #[test]
    fn front_matter_rejects_bad_values() {
        let bad_status = fm("---\nstatus: maybe\n---\n");
        assert!(bad_status.errors.iter().any(|e| e.contains("not open, parked or done")));
        let bad_id = fm("---\nstatus: open\nid: abc\n---\n");
        assert!(bad_id.errors.iter().any(|e| e.contains("not a task id")));
        let bad_dep = fm("---\nstatus: open\ndepends-on: task\n---\n");
        assert!(bad_dep.errors.iter().any(|e| e.contains("`depends-on` item `task`")));
        let bad_commit = fm("---\nstatus: done\ncommit: 12345\n---\n");
        assert!(bad_commit.errors.iter().any(|e| e.contains("not a 7–40 character hex hash")));
    }

    #[test]
    fn front_matter_id_allows_letter_suffix() {
        assert!(is_task_id("74b"));
        assert!(is_task_id("90c"));
        assert!(is_task_id("129"));
        assert!(!is_task_id("12a3"));
        assert!(!is_task_id(""));
        assert!(!is_task_id("b"));
    }

    #[test]
    fn no_front_matter_block() {
        assert!(parse_front_matter("# Tasks\n\nno header\n").is_none());
        assert!(parse_front_matter("---\nunterminated\n").is_none());
    }

    #[test]
    fn status_prose_word_extraction() {
        for (text, want) in [
            ("## Status: open — wave 0", "open"),
            ("## Status: parked (spun off task 87; unpark trigger below)", "parked"),
            ("## Status: done (294b845)", "done"),
            ("## Status: done — server half (9b3920a)", "done"),
        ] {
            let f = fm(&format!("---\nstatus: open\n---\n{text}\n"));
            let t = f.into_task_file("x.md".to_string(), false);
            assert_eq!(t.prose_status.as_deref(), Some(want));
        }
    }

    #[test]
    fn refs_task_phrasings() {
        let body = "subsumed by task 119's ownership model\nSpun off task 88 (the console client API)\ntask-88 build\nlands with task 97\n";
        let refs: Vec<&str> = extract_refs(body).iter().map(|(_, id)| *id).collect();
        assert_eq!(refs, vec!["119", "88", "88", "97"]);
    }

    #[test]
    fn refs_depends_on_list() {
        let refs: Vec<&str> = extract_refs("## Status: open — wave 6.  Depends on 118, 121")
            .iter()
            .map(|(_, id)| *id)
            .collect();
        assert_eq!(refs, vec!["118", "121"]);
    }

    #[test]
    fn refs_successor_phrasing() {
        let refs: Vec<&str> = extract_refs("parked (successor to the done task 70)")
            .iter()
            .map(|(_, id)| *id)
            .collect();
        assert_eq!(refs, vec!["70"]);
    }

    #[test]
    fn refs_skip_arc_ordinals() {
        // The gates-hardening arc numbers its members `Task N of 7`.
        let refs = extract_refs("Task 4 of 7 in the gates-hardening arc. Plan:\n");
        assert!(refs.is_empty(), "{refs:?}");
    }

    #[test]
    fn refs_of_with_non_number_is_a_reference() {
        // `of` that does not introduce a number is prose, not an ordinal.
        let refs: Vec<&str> =
            extract_refs("validation half of task 120\n").iter().map(|(_, id)| *id).collect();
        assert_eq!(refs, vec!["120"]);
    }

    #[test]
    fn refs_depends_on_task_is_not_double_counted() {
        // `Depends on task 118`: the run parser stops at the word, the
        // task scan picks it up — exactly once.
        let refs: Vec<&str> = extract_refs("## Status: open — wave 5.  Depends on task 118")
            .iter()
            .map(|(_, id)| *id)
            .collect();
        assert_eq!(refs, vec!["118"]);
    }

    #[test]
    fn malformed_block_drops_values() {
        // A value error quarantines the whole block: no check may act on
        // the sibling values.
        let f = fm("---\nid: 12\nstatus: open\npriority: high\n---\n");
        let t = f.into_task_file("x.md".to_string(), false);
        assert!(!t.malformed.is_empty());
        assert_eq!(t.id, None);
        assert!(t.status.is_empty());
    }

    #[test]
    fn refs_plural_and_lettered_ids() {
        let refs: Vec<&str> = extract_refs("fold in task 9 and tasks 106\nTask 74b split\n")
            .iter()
            .map(|(_, id)| *id)
            .collect();
        assert_eq!(refs, vec!["9", "106", "74b"]);
    }

    #[test]
    fn refs_case_insensitive_and_in_order() {
        // Both cases on one line: the earlier `Task` must not be skipped
        // because a later `task` exists.
        let refs: Vec<&str> =
            extract_refs("Task 4 of 7 but see task 46\n").iter().map(|(_, id)| *id).collect();
        assert_eq!(refs, vec!["46"]);
    }

    #[test]
    fn refs_do_not_match_inside_words() {
        assert!(extract_refs("tasking and taskschedule are not references").is_empty());
    }

    #[test]
    fn refs_line_numbers_are_one_based() {
        assert_eq!(extract_refs("nothing here\nsee task 124\n"), vec![(2, "124")]);
    }

    #[test]
    fn commit_hash_shape() {
        assert!(is_commit_hash("d773a37"));
        assert!(is_commit_hash("4a74cda"));
        assert!(is_commit_hash("4144216")); // all-digit hashes exist
        assert!(!is_commit_hash("12345")); // too short
        assert!(!is_commit_hash("ghijklm")); // not hex
    }

    #[test]
    fn commit_recovery_from_status_line() {
        assert_eq!(
            recover_commit_from_status_line(&Some("## Status: done (294b845)".to_string())),
            Some("294b845".to_string())
        );
        assert_eq!(
            recover_commit_from_status_line(&Some(
                "## Status: done (d773a37, 2026-08-27)".to_string()
            )),
            Some("d773a37".to_string())
        );
        assert_eq!(
            recover_commit_from_status_line(&Some(
                "## Status: done — server half (9b3920a) + client API".to_string()
            )),
            Some("9b3920a".to_string())
        );
        assert_eq!(recover_commit_from_status_line(&Some("## Status: done".to_string())), None);
        assert_eq!(recover_commit_from_status_line(&None), None);
    }

    #[test]
    fn todo_list_parses_entries_in_region_only() {
        let text = format!(
            "# Open tasks\n\nnarrative mentions 99. [ghost](ghost.md) here\n\n{BEGIN_MARKER}\n## 1. Section\n\n102. [a.md](a.md) — a note\n103. [b.md](b.md) — another\n\n{END_MARKER}\nfooter 104. [c.md](c.md)\n"
        );
        let listed = parse_todo_list(&text);
        assert_eq!(
            listed,
            vec![
                ("102".to_string(), "a.md".to_string(), 8),
                ("103".to_string(), "b.md".to_string(), 9),
            ]
        );
    }

    #[test]
    fn todo_list_without_markers_yields_nothing() {
        assert!(parse_todo_list("102. [a.md](a.md) — unmarked").is_empty());
    }

    #[test]
    fn todo_list_done_dir_links() {
        let text =
            format!("{BEGIN_MARKER}\n99. [done/gone.md](done/gone.md) — filed\n{END_MARKER}\n");
        assert_eq!(parse_todo_list(&text), vec![("99".to_string(), "done/gone.md".to_string(), 2)]);
    }

    #[test]
    fn unmarked_todo_is_one_finding_not_one_per_task() {
        let dir = std::env::temp_dir().join(format!("xtask-tasks-unmarked-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("todo.md"), "# Open tasks\n\n102. [a.md](a.md) — prose shape\n")
            .unwrap();
        fs::write(dir.join("a.md"), "---\nid: 102\nstatus: open\n---\n\n# a\n").unwrap();
        let tree = load(&dir).unwrap();
        let rendered: Vec<String> = index_findings(&tree).iter().map(|f| f.render()).collect();
        // The absence of the region is the finding; the per-task "not
        // listed" noise would just repeat it once per open task.
        assert!(rendered.iter().any(|r| r.contains("not reconcilable")), "{rendered:?}");
        assert!(!rendered.iter().any(|r| r.contains("not listed in todo.md")), "{rendered:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_number_collision_names_both_claimants() {
        let dir =
            std::env::temp_dir().join(format!("xtask-tasks-collision-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Entry number 12 lists b.md, but a.md's front matter claims id 12.
        fs::write(
            dir.join("todo.md"),
            format!("{BEGIN_MARKER}\n12. [b.md](b.md) — x\n{END_MARKER}\n"),
        )
        .unwrap();
        fs::write(dir.join("a.md"), "---\nid: 12\nstatus: open\n---\n\n# a\n").unwrap();
        fs::write(dir.join("b.md"), "---\nstatus: open\n---\n\n# b\n").unwrap();
        let tree = load(&dir).unwrap();
        let rendered: Vec<String> = index_findings(&tree).iter().map(|f| f.render()).collect();
        let hit = rendered.iter().any(|r| {
            r.contains("duplicate-id")
                && r.contains("12")
                && r.contains("todo.md:")
                && r.contains("a.md")
        });
        assert!(hit, "{rendered:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entry_head_parser() {
        assert_eq!(parse_entry_head("102. [a.md](a.md) — note"), Some(("102", "a.md")));
        assert_eq!(parse_entry_head("  74b. [b.md](b.md)"), Some(("74b", "b.md")));
        assert_eq!(
            parse_entry_head("99. [done/gone.md](done/gone.md)"),
            Some(("99", "done/gone.md"))
        );
        assert_eq!(parse_entry_head("## 1. Section"), None);
        assert_eq!(parse_entry_head("narrative 99. not an entry"), None);
        // A numbered prose link whose text and target differ is not an entry.
        assert_eq!(parse_entry_head("102. [the plan](plans/foo.md) says"), None);
    }

    /// Build the tree `fix()` is tested against: one listed-and-open task
    /// (with a hand-written note), one listed-but-done (stale entry), one
    /// open task with an id that is not listed, and one open task without
    /// an id that is not listed.
    fn fix_fixture(dir: &Path) -> String {
        let make = |rel: &str, text: &str| {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&p, text).unwrap();
        };
        make(
            "todo.md",
            &format!(
                "# Open tasks\n\nAudited against the tree at `abcd123`.\n\n{BEGIN_MARKER}\n## 1. Section\n\nIntro prose that must survive.\n\n10. [kept.md](kept.md) — kept task _with a note_\n11. [stale.md](stale.md) — done long ago\n\n{END_MARKER}\nfooter\n"
            ),
        );
        make("kept.md", "---\nid: 10\nstatus: open\n---\n\n# Kept task\n");
        make("stale.md", "---\nid: 11\nstatus: done\n---\n\n# Stale task\n");
        make("unlisted.md", "---\nid: 12\nstatus: open\n---\n\n# Task 12: Unlisted task\n");
        make("unlisted-noid.md", "---\nstatus: parked\n---\n\n# Unlisted, no id\n");
        let path = dir.join("todo.md");
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn fix_preserves_narrative_and_drops_stale_entries() {
        let dir = std::env::temp_dir().join(format!("xtask-tasks-fix-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let before = fix_fixture(&dir);
        let narrative_end = before.find(BEGIN_MARKER).unwrap() + BEGIN_MARKER.len();

        run_on(&dir, true).expect("fix completes");
        let after = fs::read_to_string(dir.join("todo.md")).unwrap();

        // The narrative above the begin marker is byte for byte.
        assert_eq!(
            &after[..after.find(BEGIN_MARKER).unwrap() + BEGIN_MARKER.len()],
            &before[..narrative_end]
        );
        // The stale entry is gone; the kept entry and its note survive.
        assert!(!after.contains("stale.md"));
        assert!(after.contains("10. [kept.md](kept.md) — kept task _with a note_"));
        // Section header and intro prose survive.
        assert!(after.contains("## 1. Section"));
        assert!(after.contains("Intro prose that must survive."));
        // The unlisted task with an id is appended (title prefix stripped);
        // the id-less one is not invented into the list.
        assert!(after.contains("12. [unlisted.md](unlisted.md) — Unlisted task"));
        assert!(!after.contains("[unlisted-noid.md]"));
        // The footer below the end marker is intact.
        assert!(after.ends_with(&format!("{END_MARKER}\nfooter\n")));

        // A second run changes nothing: the region now matches the tree.
        run_on(&dir, true).expect("second fix completes");
        let twice = fs::read_to_string(dir.join("todo.md")).unwrap();
        assert_eq!(twice, after);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fix_without_markers_is_an_error() {
        let dir =
            std::env::temp_dir().join(format!("xtask-tasks-fix-nomark-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("todo.md"), "# Open tasks\n\n10. [a.md](a.md)\n").unwrap();
        fs::write(dir.join("a.md"), "---\nid: 10\nstatus: open\n---\n\n# a\n").unwrap();
        let err = run_on(&dir, true).unwrap_err();
        assert!(err.to_string().contains("generated-region markers"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Build a fixture tree on disk and load it.
    fn fixture(dir: &Path) -> Tree {
        let make = |rel: &str, text: &str| {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&p, text).unwrap();
        };
        make(
            "todo.md",
            &format!(
                "# Open tasks\n\nnarrative\n\n{BEGIN_MARKER}\n## 1. S\n\n10. [open-ok.md](open-ok.md) — fine\n11. [open-done.md](open-done.md) — stale\n\n{END_MARKER}\n"
            ),
        );
        make("open-ok.md", "---\nid: 10\nstatus: open\n---\n\n# ok\nsee task 11\n");
        make("open-done.md", "---\nid: 11\nstatus: done\n---\n\n# done at top level\n");
        make("open-unlisted.md", "---\nid: 12\nstatus: open\n---\n\n# unlisted\n");
        make(
            "done/landed.md",
            "---\nid: 11\nstatus: done\ncommit: d773a37\n---\n\n# landed\n## Status: done (d773a37)\n",
        );
        load(dir).expect("fixture loads")
    }

    #[test]
    fn check_fixture_finds_the_drift() {
        let dir = std::env::temp_dir().join(format!("xtask-tasks-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let tree = fixture(&dir);
        let findings =
            file_findings(&tree).into_iter().chain(index_findings(&tree)).collect::<Vec<_>>();
        let rendered: Vec<String> = findings.iter().map(|f| f.render()).collect();

        // done at the top level, id 11 claimed twice, an unlisted open
        // task, a listed-but-done entry — all surface; the reference to
        // the (existing) id 11 in open-ok.md resolves.
        assert!(
            rendered.iter().any(|r| r.contains("open-done.md") && r.contains("misplaced")),
            "{rendered:?}"
        );
        assert!(
            rendered.iter().any(|r| r.contains("duplicate-id") && r.contains("11")),
            "{rendered:?}"
        );
        assert!(
            rendered.iter().any(|r| r.contains("open-unlisted.md") && r.contains("index-drift")),
            "{rendered:?}"
        );
        assert!(
            rendered.iter().any(|r| r.contains("open-done.md") && r.contains("which is done")),
            "{rendered:?}"
        );
        assert!(
            !rendered.iter().any(|r| r.contains("open-ok.md") && r.contains("unresolved-ref")),
            "{rendered:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The real tree, not a fixture: the structural invariants must hold
    /// on every commit. The reporting-only classes (status/prose drift,
    /// misplaced files, recoverable commits, index drift, unresolved
    /// references) are deliberately not asserted here — that is the
    /// gate's stdout, for a human's eye; what a test may fail the build
    /// for is what `--check` already exits non-zero for, plus identity
    /// defects.
    #[test]
    fn real_tree_has_no_malformed_or_duplicate_ids() {
        let dir = crate::workspace().join("tasks");
        let tree = load(&dir).expect("the real tasks/ tree loads");
        assert!(!tree.listed.is_empty(), "todo.md's marked region parsed empty");
        let findings =
            file_findings(&tree).into_iter().chain(index_findings(&tree)).collect::<Vec<_>>();
        for f in &findings {
            match f {
                Finding::Malformed { .. } | Finding::DuplicateId { .. } => {
                    panic!("structural finding in the real tree: {}", f.render())
                }
                _ => {}
            }
        }
    }

    #[test]
    fn front_matter_trailing_space_fence_keeps_body() {
        // A closing fence with trailing whitespace closes the block; the
        // body is sliced from the fence scan, not found by a second
        // search for "\n---\n" (which would find nothing here and void
        // every body-level check).
        let f = fm("---\nstatus: open\n--- \n\n## Status: done\n\nSee task 999.\n");
        assert!(f.errors.is_empty(), "{:?}", f.errors);
        assert_eq!(f.body, "\n## Status: done\n\nSee task 999.\n");
    }

    #[test]
    fn front_matter_crlf_body_is_correct() {
        // CRLF input: the offset scan keeps each line's terminator, so
        // the body starts right after the fence's `\r\n` — no fragment
        // of the fence leaks in, and the body lines are clean.
        let f = fm("---\r\nid: 1\r\nstatus: open\r\n---\r\n\r\n# body\r\nsee task 9\r\n");
        assert!(f.errors.is_empty(), "{:?}", f.errors);
        assert_eq!(f.body, "\r\n# body\r\nsee task 9\r\n");
        assert_eq!(f.body.lines().collect::<Vec<_>>(), vec!["", "# body", "see task 9"]);
    }

    #[test]
    fn front_matter_ending_on_the_fence_has_an_empty_body() {
        // A file that ends exactly on the closing fence (no trailing
        // newline): the offset clamp keeps the slice in bounds and the
        // body is simply empty.
        let f = fm("---\nid: 5\nstatus: open\n---");
        assert!(f.errors.is_empty(), "{:?}", f.errors);
        assert_eq!(f.body, "");
    }

    #[test]
    fn unresolved_front_matter_ref_renders_without_a_line() {
        let f = Finding::UnresolvedRef { file: "a.md".into(), line: 0, id: "99".into() };
        assert!(f.render().contains("a.md (front matter)"), "{}", f.render());
        let g = Finding::UnresolvedRef { file: "a.md".into(), line: 12, id: "99".into() };
        assert!(g.render().contains("a.md:12"), "{}", g.render());
    }

    #[test]
    fn refs_depends_on_lettered_ids() {
        let refs = extract_refs("Depends on 74b, 80\n");
        assert_eq!(refs, vec![(1, "74b"), (1, "80")]);
    }

    #[test]
    fn refs_uppercase_suffix_is_not_read_as_the_bare_number() {
        // `Task 74B` is not an id shape; it must not silently resolve
        // as task 74.
        let refs = extract_refs("see Task 74B for the layout\n");
        assert_eq!(refs, vec![(1, "74B")]);
    }

    #[test]
    fn refs_arc_ordinal_is_case_insensitive() {
        assert!(extract_refs("Task 4 Of 7: the gate\n").is_empty());
    }

    #[test]
    fn moved_task_entry_reports_drift_not_duplicate() {
        // A task moved to done/ leaves its bare-filename entry behind:
        // that is index drift, not a duplicate id (claimants carry the
        // moved rel path, the entry the bare name).
        let dir = std::env::temp_dir().join(format!("xtask-tasks-moved-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let make = |rel: &str, text: &str| {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&p, text).unwrap();
        };
        make(
            "todo.md",
            &format!(
                "# t\n\n{BEGIN_MARKER}\n## 1. S\n\n20. [moved.md](moved.md) — stale\n\n{END_MARKER}\n"
            ),
        );
        make("done/moved.md", "---\nid: 20\nstatus: done\ncommit: d773a37\n---\n\n# moved\n");
        let tree = load(&dir).expect("loads");
        let findings =
            file_findings(&tree).into_iter().chain(index_findings(&tree)).collect::<Vec<_>>();
        assert!(findings.iter().any(|f| matches!(f, Finding::IndexDrift { .. })), "{findings:?}");
        assert!(!findings.iter().any(|f| matches!(f, Finding::DuplicateId { .. })), "{findings:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fix_aborts_on_malformed_front_matter() {
        // The writer's trust base is the parse: a malformed file looks
        // done (its values are dropped), so reconciling would delete its
        // entry and note. The run must stop and leave todo.md untouched.
        let dir = std::env::temp_dir().join(format!("xtask-tasks-fix-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("todo.md"),
            format!("# t\n\n{BEGIN_MARKER}\n## 1. S\n\n30. [broken.md](broken.md) — note that must survive\n\n{END_MARKER}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("broken.md"),
            "---\nid: 30\nstatus: open\npriority: high\n---\n\n# broken\n",
        )
        .unwrap();
        let err = run_on(&dir, true).unwrap_err();
        assert!(err.to_string().contains("malformed"), "{err}");
        let after = fs::read_to_string(dir.join("todo.md")).unwrap();
        assert!(
            after.contains("30. [broken.md](broken.md) — note that must survive"),
            "fix touched todo.md: {after}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fix_subheader_ends_the_block_above_it() {
        // A `### ` sub-header is a boundary like `## `: the block above
        // it is dropped whole, the sub-header and what follows survive.
        let dir = std::env::temp_dir().join(format!("xtask-tasks-fix-sub-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("todo.md"),
            format!("# t\n\n{BEGIN_MARKER}\n## 1. S\n\n40. [stale.md](stale.md) — gone\n### kept sub-section\nprose\n\n{END_MARKER}\n"),
        )
        .unwrap();
        fs::write(dir.join("stale.md"), "---\nid: 40\nstatus: done\n---\n\n# stale\n").unwrap();
        run_on(&dir, true).expect("fix completes");
        let after = fs::read_to_string(dir.join("todo.md")).unwrap();
        assert!(!after.contains("stale.md"), "{after}");
        assert!(after.contains("### kept sub-section\nprose"), "{after}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_fixture_clean_tree_reports_nothing() {
        let dir = std::env::temp_dir().join(format!("xtask-tasks-clean-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let make = |rel: &str, text: &str| {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&p, text).unwrap();
        };
        make(
            "todo.md",
            &format!(
                "# Open tasks\n\n{BEGIN_MARKER}\n## 1. S\n\n10. [only.md](only.md) — it\n\n{END_MARKER}\n"
            ),
        );
        make("only.md", "---\nid: 10\nstatus: open\n---\n\n# only\n");
        let tree = load(&dir).expect("loads");
        let findings =
            file_findings(&tree).into_iter().chain(index_findings(&tree)).collect::<Vec<_>>();
        assert!(findings.is_empty(), "{findings:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}

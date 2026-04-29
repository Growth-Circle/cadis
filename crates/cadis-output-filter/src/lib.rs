use regex::Regex;

/// Result of filtering command output.
pub struct FilterResult {
    pub filtered: String,
    pub original_bytes: usize,
    pub filtered_bytes: usize,
    pub savings_pct: f64,
}

/// Route `raw` output through the appropriate filter for `command`.
pub fn filter_output(command: &str, raw: &str) -> FilterResult {
    let clean = strip_ansi(raw);
    let filtered = match detect_command(command) {
        CommandKind::CargoTest => filter_cargo_test(&clean),
        CommandKind::CargoBuild => filter_cargo_build(&clean),
        CommandKind::CargoClippy => filter_cargo_clippy(&clean),
        CommandKind::GitStatus => filter_git_status(&clean),
        CommandKind::GitDiff => filter_git_diff(&clean),
        CommandKind::GitLog => filter_git_log(&clean),
        CommandKind::NpmTest => filter_npm_test(&clean),
        CommandKind::Generic => filter_generic(&clean),
    };
    let original_bytes = raw.len();
    let filtered_bytes = filtered.len();
    let savings_pct = if original_bytes == 0 {
        0.0
    } else {
        (1.0 - filtered_bytes as f64 / original_bytes as f64) * 100.0
    };
    FilterResult {
        filtered,
        original_bytes,
        filtered_bytes,
        savings_pct,
    }
}

// ---------------------------------------------------------------------------
// Pipeline stages
// ---------------------------------------------------------------------------

fn strip_ansi(input: &str) -> String {
    let re = Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").expect("valid regex");
    re.replace_all(input, "").into_owned()
}

fn dedup_lines(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let cur = lines[i];
        let mut count = 1usize;
        while i + count < lines.len() && lines[i + count] == cur {
            count += 1;
        }
        if count >= 3 {
            out.push(format!("{cur} (x{count})"));
        } else {
            for _ in 0..count {
                out.push(cur.to_string());
            }
        }
        i += count;
    }
    out.join("\n")
}

fn truncate_output(input: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() <= max_lines {
        return input.to_string();
    }
    let head = 40.min(lines.len());
    let tail = 20.min(lines.len().saturating_sub(head));
    let tail_start = lines.len() - tail;
    // Collect important lines from the omitted middle section
    let important: Vec<&str> = lines[head..tail_start]
        .iter()
        .copied()
        .filter(|l| is_important(l))
        .collect();
    let omitted = lines.len() - head - tail - important.len();
    let mut result: Vec<String> = lines[..head].iter().map(|s| s.to_string()).collect();
    result.push(format!("... ({omitted} lines omitted)"));
    result.extend(important.iter().map(|s| s.to_string()));
    result.extend(lines[tail_start..].iter().map(|s| s.to_string()));
    result.join("\n")
}

// ---------------------------------------------------------------------------
// Important-line detection
// ---------------------------------------------------------------------------

fn is_important(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("error")
        || lower.contains("failed")
        || lower.contains("panic")
        || lower.contains("warn")
}

// ---------------------------------------------------------------------------
// Command-specific filters
// ---------------------------------------------------------------------------

fn filter_cargo_test(raw: &str) -> String {
    let mut out = Vec::new();
    for line in raw.lines() {
        if line.starts_with("test result:") || line.contains("FAILED") || is_important(line) {
            out.push(line);
        }
    }
    if out.is_empty() {
        // fallback: keep last line (usually the summary)
        if let Some(last) = raw.lines().last() {
            out.push(last);
        }
    }
    out.join("\n")
}

fn filter_cargo_build(raw: &str) -> String {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("error")
            || trimmed.starts_with("warning")
            || trimmed.starts_with("Finished")
            || is_important(line)
        {
            out.push(line);
        }
    }
    if out.is_empty() {
        if let Some(last) = raw.lines().last() {
            out.push(last);
        }
    }
    out.join("\n")
}

fn filter_cargo_clippy(raw: &str) -> String {
    // Same strategy as cargo build — warnings and errors are the signal.
    filter_cargo_build(raw)
}

fn filter_git_status(raw: &str) -> String {
    // Strip hint lines (start with whitespace + "(use ")
    raw.lines()
        .filter(|l| !l.trim_start().starts_with("(use "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn filter_git_diff(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    if lines.len() <= 200 {
        return raw.to_string();
    }
    let mut out = Vec::new();
    let mut hunk_changed = 0usize;
    for line in &lines {
        if line.starts_with("diff --git")
            || line.starts_with("---")
            || line.starts_with("+++")
            || line.starts_with("@@")
        {
            out.push(*line);
            hunk_changed = 0;
        } else if (line.starts_with('+') || line.starts_with('-')) && hunk_changed < 5 {
            out.push(*line);
            hunk_changed += 1;
        } else if is_important(line) {
            out.push(*line);
        }
    }
    out.join("\n")
}

fn filter_git_log(raw: &str) -> String {
    // Already compact — just pass through after ansi strip.
    raw.to_string()
}

fn filter_npm_test(raw: &str) -> String {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("FAIL")
            || trimmed.starts_with("✕")
            || trimmed.starts_with("×")
            || is_important(line)
        {
            out.push(line);
        }
    }
    if out.is_empty() {
        if let Some(last) = raw.lines().last() {
            out.push(last);
        }
    }
    out.join("\n")
}

fn filter_generic(raw: &str) -> String {
    let deduped = dedup_lines(raw);
    truncate_output(&deduped, 100)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum CommandKind {
    CargoTest,
    CargoBuild,
    CargoClippy,
    GitStatus,
    GitDiff,
    GitLog,
    NpmTest,
    Generic,
}

fn detect_command(command: &str) -> CommandKind {
    let cmd = command.to_lowercase();
    if cmd.contains("cargo test") || cmd.contains("cargo nextest") {
        CommandKind::CargoTest
    } else if cmd.contains("cargo clippy") {
        CommandKind::CargoClippy
    } else if cmd.contains("cargo build") || cmd.contains("cargo check") {
        CommandKind::CargoBuild
    } else if cmd.contains("git status") {
        CommandKind::GitStatus
    } else if cmd.contains("git diff") {
        CommandKind::GitDiff
    } else if cmd.contains("git log") {
        CommandKind::GitLog
    } else if cmd.contains("npm test") || cmd.contains("npx jest") || cmd.contains("npx vitest") {
        CommandKind::NpmTest
    } else {
        CommandKind::Generic
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi() {
        let input = "\x1b[32mok\x1b[0m some text \x1b[1;31merror\x1b[0m";
        let out = strip_ansi(input);
        assert_eq!(out, "ok some text error");
        assert!(!out.contains("\x1b"));
    }

    #[test]
    fn test_dedup_lines() {
        let input = "a\nb\nb\nb\nb\nc";
        let out = dedup_lines(input);
        assert!(out.contains("b (x4)"));
        assert!(out.contains("a"));
        assert!(out.contains("c"));
        // two consecutive identical lines should NOT be collapsed
        let input2 = "x\nx\ny";
        let out2 = dedup_lines(input2);
        assert_eq!(out2, "x\nx\ny");
    }

    #[test]
    fn test_truncate() {
        let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
        let input = lines.join("\n");
        let out = truncate_output(&input, 60);
        assert!(out.contains("... (140 lines omitted)"));
        assert!(out.contains("line 0"));
        assert!(out.contains("line 199"));
        // short input unchanged
        assert_eq!(truncate_output("short", 100), "short");
    }

    #[test]
    fn test_cargo_test_filter() {
        let raw = "\
running 3 tests
test foo ... ok
test bar ... ok
test baz ... FAILED

failures:
    baz panicked at 'assertion failed'

test result: FAILED. 2 passed; 1 failed; 0 ignored";
        let out = filter_cargo_test(raw);
        assert!(out.contains("test result:"));
        assert!(out.contains("FAILED"));
        assert!(!out.contains("test foo ... ok"));
    }

    #[test]
    fn test_cargo_build_filter() {
        let raw = "\
   Compiling foo v0.1.0
   Compiling bar v0.2.0
warning: unused variable `x`
error[E0308]: mismatched types
   Compiling baz v0.3.0
Finished dev [unoptimized + debuginfo] target(s)";
        let out = filter_cargo_build(raw);
        assert!(out.contains("warning: unused variable"));
        assert!(out.contains("error[E0308]"));
        assert!(out.contains("Finished"));
        assert!(!out.contains("Compiling foo"));
    }

    #[test]
    fn test_filter_output_routing() {
        let r = filter_output("cargo test --release", "test result: ok. 5 passed");
        assert!(r.filtered.contains("test result:"));

        let r2 = filter_output("git status", "On branch main\n  (use \"git add\" to stage)");
        assert!(!r2.filtered.contains("(use "));

        let r3 = filter_output("echo hello", "hello");
        assert_eq!(r3.filtered, "hello");
    }

    #[test]
    fn test_error_lines_preserved() {
        // Important lines must survive generic filtering even in the omitted range
        let mut lines: Vec<String> = (0..150).map(|i| format!("noise {i}")).collect();
        lines[75] = "FATAL error: something broke".to_string();
        lines[100] = "panic at the disco".to_string();
        let input = lines.join("\n");
        let out = filter_output("some-cmd", &input);
        assert!(out.filtered.contains("FATAL error: something broke"));
        assert!(out.filtered.contains("panic at the disco"));
        assert!(out.savings_pct > 0.0);
    }
}

//! Files the user writes out on purpose (P08): the HTML scorecard, the
//! session history CSV and the database backup.
//!
//! The user types (or keeps) a target path in the overlay; this module
//! checks it before anything is written. Nothing is ever overwritten: an
//! existing file is reported, not replaced. Every write happens on a
//! button press, never while typing.

use std::path::{Path, PathBuf};

/// Where exports go by default: `Documents\Radha Typing Tutor`, or the
/// app's data folder when the system has no Documents folder.
#[must_use]
pub fn default_dir(data_dir: &Path) -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| u.document_dir().map(Path::to_path_buf))
        .map_or_else(
            || data_dir.join("exports"),
            |d| d.join("Radha Typing Tutor"),
        )
}

/// A file name made safe for Windows from free text ("Exercise 3 · Home").
#[must_use]
pub fn file_stem(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed: String = out.trim_matches('-').chars().take(60).collect();
    if trimmed.is_empty() {
        "radha".to_owned()
    } else {
        trimmed
    }
}

/// Check a user-entered target: absolute, with one of `extensions`
/// (lower-case, no dot), and not an existing file.
///
/// # Errors
///
/// A sentence for the overlay explaining what to change.
pub fn validate_target(text: &str, extensions: &[&str]) -> Result<PathBuf, String> {
    let text = text.trim().trim_matches('"');
    if text.is_empty() {
        return Err("Enter where to save the file.".to_owned());
    }
    let path = PathBuf::from(text);
    if !path.is_absolute() {
        return Err("Use a full path, for example C:\\Users\\you\\Documents\\file.".to_owned());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !extensions.contains(&ext.as_str()) {
        return Err(format!(
            "The file name must end in .{}.",
            extensions.join(" or .")
        ));
    }
    if path.exists() {
        return Err(format!(
            "{} already exists. Choose another name; nothing was overwritten.",
            path.display()
        ));
    }
    Ok(path)
}

/// Create `path`'s folder if needed and write `bytes` to a new file.
///
/// # Errors
///
/// The I/O error (including "already exists": the file is never replaced).
pub fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// One CSV field, quoted when it needs to be. A leading `= + - @` is
/// prefixed with `'` so a spreadsheet never runs it as a formula.
#[must_use]
pub fn csv_field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s.to_owned()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

/// The session history as CSV (UTF-8 with BOM, so Excel reads Hindi).
#[must_use]
pub fn history_csv(rows: &[radha_db::SessionLine]) -> String {
    let mut out = String::from(
        "\u{feff}completed_at,product,exercise,title,mode,gross_wpm,net_wpm,accuracy,seconds\r\n",
    );
    for r in rows {
        let fields = [
            csv_field(&r.completed_at),
            csv_field(&r.product_name),
            r.exercise_number.to_string(),
            csv_field(&r.exercise_title),
            csv_field(r.mode.as_deref().unwrap_or("")),
            format!("{:.1}", r.gross_wpm),
            format!("{:.1}", r.net_wpm),
            format!("{:.1}", r.accuracy),
            r.elapsed_secs.to_string(),
        ];
        out.push_str(&fields.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_validated_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let ok = temp.path().join("out").join("card.HTML");
        assert_eq!(
            validate_target(&ok.display().to_string(), &["html", "htm"]),
            Ok(ok.clone())
        );
        assert!(validate_target("", &["html"]).is_err());
        assert!(validate_target("card.html", &["html"]).is_err(), "relative");
        let wrong = temp.path().join("card.exe");
        assert!(validate_target(&wrong.display().to_string(), &["html"])
            .unwrap_err()
            .contains(".html"));
        write_new(&ok, b"one").unwrap();
        assert!(validate_target(&ok.display().to_string(), &["html"])
            .unwrap_err()
            .contains("already exists"));
        assert!(write_new(&ok, b"two").is_err(), "never overwrites");
        assert_eq!(std::fs::read(&ok).unwrap(), b"one");
    }

    #[test]
    fn file_stems_are_windows_safe() {
        assert_eq!(file_stem("Exercise 3 · Home: Row?"), "Exercise-3-Home-Row");
        assert!(
            file_stem("नमस्ते दुनिया").starts_with("नमस"),
            "Hindi letters stay"
        );
        assert_eq!(file_stem(":::"), "radha");
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("=HYPERLINK(1)"), "'=HYPERLINK(1)");
        let rows = [radha_db::SessionLine {
            session_id: 1,
            completed_at: "2026-10-25T10:01:00Z".into(),
            product_slug: "english-classic".into(),
            product_name: "English Typing Mastery".into(),
            exercise_id: 7,
            exercise_number: 3,
            exercise_title: "Home, row".into(),
            mode: Some("test".into()),
            gross_wpm: 41.2,
            net_wpm: 39.0,
            accuracy: 97.5,
            elapsed_secs: 60,
        }];
        let csv = history_csv(&rows);
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert!(lines[0].starts_with("\u{feff}completed_at,"));
        assert_eq!(
            lines[1],
            "2026-10-25T10:01:00Z,English Typing Mastery,3,\"Home, row\",test,41.2,39.0,97.5,60"
        );
    }
}

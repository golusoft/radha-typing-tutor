//! English Exam Master: one consolidated pool of 500 passages, IDs and
//! titles from Golu's title bank (`data/exam-500-english-titles.md`, 20
//! profiles x 25).
//!
//! * A passage that exists in the Radha 500 drafts (`--radha500`, one
//!   `<ID>.json` per passage in profile folders) under the same ID and title
//!   is reused: its typing sections, typography folded to US-keyboard ASCII,
//!   cut at a sentence end to at most `EXAM_MAX_WORDS` words. Status
//!   provisional (the drafts say `content_status: draft`).
//! * Every other ID is written from its authored brief
//!   (`data/exam-english-briefs.txt`: a topic phrase and four facts per
//!   title, plus a fictional office per profile) by [`compose`]: fictional
//!   names, real calendar dates with their true weekdays, reference numbers,
//!   and counts whose totals add up. Status generated-review-pending.
//!
//! No sentence of six or more words appears in two passages of the pool;
//! the importer re-rolls a passage until that holds.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{bail, Context};
use radha_content::codec::sha256_hex;
use radha_content::import::{fold_typography, sentence_ends, trim_to_words, ImportedExercise};
use radha_content::{quality, ContentStatus, EXAM_MAX_WORDS};
use radha_layouts::LayoutId;

use crate::rng::Rng;

/// Minimum words of a passage (a 10–15 minute test).
pub const MIN_WORDS: usize = 450;
/// Minimum characters (key depressions) of a data-entry passage.
pub const MIN_KEYS_DATA_ENTRY: usize = 2000;
/// Profiles whose passages are data-entry tests (key depressions).
pub const DATA_ENTRY: [&str; 2] = ["CGL-DEST", "EPFO"];

/// Pipeline label of written passages.
pub const PIPELINE: &str = "radha-exam-briefs:data/exam-english-briefs.txt";

/// One row of the title bank.
#[derive(Debug, Clone)]
pub struct TitleRow {
    /// `CHSL-001`.
    pub id: String,
    /// `CHSL`.
    pub code: String,
    /// `SSC CHSL — LDC / JSA`.
    pub profile: String,
    /// Passage title.
    pub title: String,
}

/// Parse the title bank: `## CODE — Display — Context` then `ID | Title`.
///
/// # Errors
///
/// When the bank is not 20 profiles of 25 rows with matching ids.
pub fn parse_titles(md: &str) -> anyhow::Result<Vec<TitleRow>> {
    let mut rows = Vec::new();
    let mut profile: Option<(String, String)> = None;
    for line in md.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            // `CODE — Display name — Context`: the context is not kept.
            let parts: Vec<&str> = head.split(" — ").map(str::trim).collect();
            profile = (parts.len() >= 3)
                .then(|| (parts[0].to_owned(), parts[1..parts.len() - 1].join(" — ")));
            continue;
        }
        let Some((code, display)) = &profile else {
            continue;
        };
        let Some((id, title)) = line.split_once(" | ") else {
            continue;
        };
        let id = id.trim();
        if !id.starts_with(&format!("{code}-")) {
            continue;
        }
        rows.push(TitleRow {
            id: id.to_owned(),
            code: code.clone(),
            profile: display.clone(),
            title: title.trim().to_owned(),
        });
    }
    if rows.len() != 500 {
        bail!("title bank has {} rows (500 expected)", rows.len());
    }
    let ids: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    if ids.len() != 500 {
        bail!("title bank ids are not unique");
    }
    for chunk in rows.chunks(25) {
        for (k, r) in chunk.iter().enumerate() {
            if r.id != format!("{}-{:03}", r.code, k + 1) {
                bail!("title bank row {} out of order", r.id);
            }
        }
    }
    Ok(rows)
}

/// A fictional office a profile's passages are set in.
#[derive(Debug, Clone, Default)]
pub struct Setting {
    /// Office name.
    pub org: String,
    /// Town.
    pub place: String,
    /// Sections of the office.
    pub sections: Vec<String>,
    /// Job title of the typist in the story.
    pub role: String,
    /// The public the office serves.
    pub party: String,
    /// Reference prefix.
    pub code: String,
}

/// One title's brief.
#[derive(Debug, Clone, Default)]
pub struct Brief {
    /// A noun phrase for the topic, used inside sentences.
    pub topic: String,
    /// Four facts specific to the title.
    pub facts: Vec<String>,
}

/// Parse the briefs file (see the file header for its format).
///
/// # Errors
///
/// When a block is malformed.
pub fn parse_briefs(
    text: &str,
) -> anyhow::Result<(HashMap<String, Setting>, HashMap<String, Brief>)> {
    let mut settings: HashMap<String, Setting> = HashMap::new();
    let mut briefs: HashMap<String, Brief> = HashMap::new();
    let mut cur_setting: Option<String> = None;
    let mut cur_brief: Option<String> = None;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(code) = line.strip_prefix("@profile ") {
            let code = code.trim().to_owned();
            settings.insert(code.clone(), Setting::default());
            cur_setting = Some(code);
            cur_brief = None;
        } else if let Some(rest) = line.strip_prefix('#') {
            let (id, topic) = rest
                .split_once('|')
                .with_context(|| format!("briefs line {}: `#ID | topic` expected", n + 1))?;
            let id = id.trim().to_owned();
            if briefs.contains_key(&id) {
                bail!("briefs line {}: {id} appears twice", n + 1);
            }
            briefs.insert(
                id.clone(),
                Brief {
                    topic: topic.trim().to_owned(),
                    facts: Vec::new(),
                },
            );
            cur_brief = Some(id);
            cur_setting = None;
        } else if let Some(id) = &cur_brief {
            let fact = line.strip_prefix("- ").unwrap_or(line).to_owned();
            if let Some(b) = briefs.get_mut(id) {
                b.facts.push(fact);
            }
        } else if let Some(code) = &cur_setting {
            let (k, v) = line
                .split_once(':')
                .with_context(|| format!("briefs line {}: `key: value` expected", n + 1))?;
            let v = v.trim().to_owned();
            let s = settings.get_mut(code).context("setting")?;
            match k.trim() {
                "org" => s.org = v,
                "place" => s.place = v,
                "sections" => s.sections = v.split(';').map(|x| x.trim().to_owned()).collect(),
                "role" => s.role = v,
                "party" => s.party = v,
                "code" => s.code = v,
                other => bail!("briefs line {}: unknown key {other:?}", n + 1),
            }
        } else {
            bail!("briefs line {}: text outside a block", n + 1);
        }
    }
    for (id, b) in &briefs {
        if b.facts.len() < 4 || b.topic.is_empty() {
            bail!("brief {id} needs a topic and four facts");
        }
    }
    for (code, s) in &settings {
        if s.org.is_empty()
            || s.sections.len() < 3
            || s.role.is_empty()
            || s.party.is_empty()
            || s.code.is_empty()
        {
            bail!("profile setting {code} is incomplete");
        }
    }
    Ok((settings, briefs))
}

/// A Radha 500 draft passage: id, title and the typing sections' text.
struct DraftFile {
    exercise_id: String,
    passage_title: String,
    sections: Vec<String>,
}

fn parse_draft(bytes: &[u8]) -> anyhow::Result<DraftFile> {
    let v: serde_json::Value = serde_json::from_slice(bytes)?;
    let s = |k: &str| -> anyhow::Result<String> {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .with_context(|| format!("no {k}"))
    };
    let sections = v
        .get("typing_content")
        .and_then(serde_json::Value::as_array)
        .context("no typing_content")?
        .iter()
        .map(|sec| {
            sec.get("text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .context("a typing section without text")
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(DraftFile {
        exercise_id: s("exercise_id")?,
        passage_title: s("passage_title")?,
        sections,
    })
}

/// Reusable drafts by id: (text, relative path, sha256 of the file).
pub type Drafts = HashMap<String, (String, String, String)>;

/// Reusable drafts by id, and notes on what was folded or skipped.
///
/// # Errors
///
/// When the drafts folder cannot be read.
pub fn load_drafts(dir: &Path, rows: &[TitleRow]) -> anyhow::Result<(Drafts, Vec<String>)> {
    let titles: HashMap<&str, &str> = rows
        .iter()
        .map(|r| (r.id.as_str(), r.title.as_str()))
        .collect();
    let mut out = HashMap::new();
    let mut notes = Vec::new();
    let mut folded_total: Vec<(char, usize)> = Vec::new();
    let mut folders: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let mut files: Vec<_> = fs::read_dir(&folder)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for path in files {
            let bytes = fs::read(&path)?;
            let rel = format!(
                "radha-500:{}/{}",
                folder.file_name().unwrap_or_default().to_string_lossy(),
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            let draft: DraftFile = match parse_draft(&bytes) {
                Ok(d) => d,
                Err(e) => {
                    notes.push(format!("SKIPPED {rel}: {e}"));
                    continue;
                }
            };
            match titles.get(draft.exercise_id.as_str()) {
                Some(t) if *t == draft.passage_title.trim() => {}
                Some(t) => {
                    notes.push(format!(
                        "SKIPPED {rel}: title {:?} is not {t:?}",
                        draft.passage_title
                    ));
                    continue;
                }
                None => {
                    notes.push(format!(
                        "SKIPPED {rel}: id {} is not in the title bank",
                        draft.exercise_id
                    ));
                    continue;
                }
            }
            let joined = draft
                .sections
                .iter()
                .map(|s| s.trim().to_owned())
                .collect::<Vec<_>>()
                .join("\n\n");
            let (folded, changes) = fold_typography(&joined);
            for (c, n) in changes {
                match folded_total.iter_mut().find(|(f, _)| *f == c) {
                    Some((_, t)) => *t += n,
                    None => folded_total.push((c, n)),
                }
            }
            // One paragraph per line.
            let paragraphs: Vec<String> = folded
                .split("\n\n")
                .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|p| !p.is_empty())
                .collect();
            let text = trim_to_words(
                &paragraphs.join("\n"),
                EXAM_MAX_WORDS,
                sentence_ends(LayoutId::Qwerty),
            );
            out.insert(draft.exercise_id.clone(), (text, rel, sha256_hex(&bytes)));
        }
    }
    folded_total.sort();
    notes.push(format!(
        "typography folded to ASCII in the drafts: {}",
        folded_total
            .iter()
            .map(|(c, n)| format!("U+{:04X} x{n}", u32::from(*c)))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    Ok((out, notes))
}

// --- the composer -------------------------------------------------------------------

const FIRST: [&str; 64] = [
    "Aarav", "Meera", "Kabir", "Nisha", "Rohan", "Ananya", "Vikram", "Priya", "Arjun", "Sana",
    "Devika", "Imran", "Lata", "Manoj", "Farida", "Harish", "Kavya", "Naveen", "Pooja", "Rahul",
    "Sunita", "Tarun", "Usha", "Yash", "Zoya", "Bhavna", "Chetan", "Deepa", "Gaurav", "Hema",
    "Irfan", "Jaya", "Karan", "Leela", "Mohit", "Neha", "Omkar", "Pallavi", "Qasim", "Ritu",
    "Suresh", "Tanvi", "Uday", "Varsha", "Wasim", "Alka", "Bharat", "Charu", "Dinesh", "Esha",
    "Firoz", "Geeta", "Hitesh", "Indu", "Jatin", "Komal", "Lalit", "Madhu", "Nitin", "Pankaj",
    "Rekha", "Sameer", "Trisha", "Vinod",
];
const LAST: [&str; 48] = [
    "Sharma",
    "Verma",
    "Khan",
    "Iyer",
    "Das",
    "Mehta",
    "Rao",
    "Nair",
    "Gupta",
    "Bose",
    "Sinha",
    "Patel",
    "Joshi",
    "Kulkarni",
    "Mishra",
    "Pandey",
    "Reddy",
    "Saxena",
    "Tiwari",
    "Yadav",
    "Chauhan",
    "Dutta",
    "Ghosh",
    "Kapoor",
    "Malhotra",
    "Menon",
    "Pillai",
    "Qureshi",
    "Rathore",
    "Shukla",
    "Thakur",
    "Vyas",
    "Agarwal",
    "Banerjee",
    "Chopra",
    "Dixit",
    "Fernandes",
    "Gill",
    "Hegde",
    "Jain",
    "Kohli",
    "Lal",
    "Naidu",
    "Ojha",
    "Prasad",
    "Rawat",
    "Sethi",
    "Walia",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Day of the week (0 = Monday) of a Gregorian date.
#[must_use]
pub fn weekday(y: i64, m: i64, d: i64) -> usize {
    // Sakamoto's method: 0 = Sunday.
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let idx = usize::try_from(m - 1).unwrap_or(0);
    let s = (y + y / 4 - y / 100 + y / 400 + t[idx] + d).rem_euclid(7);
    usize::try_from((s + 6) % 7).unwrap_or(0)
}

const OPENERS: [&str; 24] = [
    "On {weekday}, {date}, {name1}, a {role} in the {section} of the {org}, {place}, opened the morning file on {topic}.",
    "The first entry in the {section} on {weekday}, {date}, concerned {topic}, and {name1}, the {role} on duty, took charge of it at {time}.",
    "At {time} on {date}, the {section} of the {org} in {place} received the papers on {topic}, and {name1} was asked to process them.",
    "{name1} reached the {section} of the {org} a little before {time} on {weekday}, {date}, to find a folder on {topic} waiting on the desk.",
    "When the {org} in {place} opened on {weekday}, {date}, the day's work for {name1}, its {role}, began with {topic}.",
    "The note pinned to the {section} board on {date} read simply, \"{title}\", and {name1} started on it at {time}.",
    "By {time} on {weekday}, {date}, {name1} had cleared the inward tray of the {section} and turned to {topic}.",
    "In the fictional {org} at {place}, {topic} falls to the {section}, and on {date} the work was assigned to {name1}.",
    "{name1} had worked as a {role} at the {org}, {place}, for three years, and on {weekday}, {date}, the task in hand was {topic}.",
    "The {section} of the {org} keeps a fixed routine for {topic}, and on {date} {name1} followed it from {time} onwards.",
    "A request from {name2} arrived in the {section} at {time} on {weekday}, {date}: {name1} was to take up {topic} before noon.",
    "Every working day at the {org} in {place} produces some record of {topic}; on {weekday}, {date}, that record was in the hands of {name1}.",
    "The office diary of the {section} for {date} shows that {name1}, {role}, began {topic} at {time} and finished well before closing.",
    "{name2}, who supervises the {section} of the {org}, handed {name1} the papers on {topic} at {time} on {weekday}, {date}.",
    "It was a quiet {weekday} morning, {date}, at the {org} in {place} when {name1} opened the register kept for {topic}.",
    "{name1}'s first assignment on {date} at the {section} of the {org} was {topic}, a task that leaves little room for guesswork.",
    "The {org} in {place} had set aside {weekday}, {date}, for {topic}, and the {section} placed {name1} in charge of the typing.",
    "Shortly after {time} on {date}, {name1} logged in at the {section} of the {org} and pulled up the file on {topic}.",
    "For the {section} of the {org}, {date} was the day fixed for {topic}, and {name1}, its {role}, prepared the desk at {time}.",
    "On the morning of {weekday}, {date}, {name2} asked {name1} to give full attention to {topic} until the work was complete.",
    "The work sheet issued to {name1} at the {org}, {place}, on {date} listed a single heading: {topic}.",
    "A {role} at the {org} in {place}, {name1} began {weekday}, {date}, with a fresh bundle of papers on {topic}.",
    "{name1} and {name2} of the {section} met at {time} on {date} to divide the day's work, and {topic} came to {name1}.",
    "There were no visitors at the {section} counter at {time} on {weekday}, {date}, so {name1} settled down to {topic}.",
];

// Every context sentence names a person or a reference of its own passage,
// so no two passages share one.
const CONTEXTS: [&str; 16] = [
    "The {org} is a fictional office, {name1} is an invented officer, and every number and date in this practice passage is made up.",
    "Nothing in this account comes from a real file; {name1}, {name2} and the {party}s around them exist only for typing practice.",
    "This is a practice story: the {org} at {place}, {name2} and reference {ref} are fictional, and so are the other records.",
    "All the details that follow, from file {ref} to the names of the {party}s, are invented for this exercise.",
    "{name1}, {name3} and the figures in this passage are fictional; they are chosen only to give realistic typing material.",
    "Readers should treat the {org} and its officer {name2} as imaginary; the procedure shown here is a teaching example, not an official rule.",
    "None of the people described below, including {name3}, is real, and the routine of the {section} is an example made for practice.",
    "The passage describes an invented office at {place}; {name1}, its {party}s and file {ref2} are all fictional.",
    "As a practice text, this account follows {name2} through a fictional office and records, such as {ref}, that belong to no real person.",
    "Every record mentioned in this passage is fictional, including file {ref2} of the {section}.",
    "The {section} where {name1} works is imaginary, and its daily routine is offered only as clear, formal typing material.",
    "Because this is practice material, {name3}, the {org} and every {party} named in it are invented for the purpose.",
    "Like the others in this set, this passage about {name2} uses a fictional office at {place} and invented names, dates and numbers.",
    "The {org} does not exist, and neither does {name1}; the setting is only for practising accurate typing of names, numbers and formal sentences.",
    "Each figure below is fictional, and what {name2} does in the {section} is an illustration rather than a statement of official practice.",
    "The names, dates and references in this passage, such as {ref}, were made up for practice and point to no real case or person.",
];

const CONNECT: [&str; 12] = [
    "First, ",
    "To begin with, ",
    "Next, ",
    "After that, ",
    "In addition, ",
    "Equally important, ",
    "At the same time, ",
    "Later in the morning, ",
    "As the work went on, ",
    "Before moving on, ",
    "Throughout the task, ",
    "Finally, ",
];

const ELABORATE: [&str; 36] = [
    "{name2} checked the step against reference {ref} before initialling the margin at {time2}.",
    "For the entry under {ref}, this meant counting {k} pages twice before the file moved on.",
    "{name1} wrote the reason for the step in the remarks column, so that {name2} could follow it later.",
    "The rule saved time on {weekday}: {k} entries were cleared without a single query from the next desk.",
    "In the {section}, a short note beside reference {ref2} records exactly when this was done.",
    "When {name3} asked why the step mattered, {name1} pointed to file {ref}, where it had prevented a wrong entry.",
    "This was applied to all {n} papers in the tray, and each one received the same treatment.",
    "{name2} later compared the result with the master list and found all {k} lines in agreement.",
    "Doing so took about {m} minutes, but it meant that the record under {ref2} could be trusted without a second search.",
    "No detail was filled in from memory; where a value was missing, the field under {ref} was left blank and flagged.",
    "A copy of the note went to {name3} in the {section2}, who keeps the reference list for the {org}.",
    "The same check was repeated at {time2}, when {n} further papers reached the desk.",
    "{name1} read each figure aloud while {name2} followed the original, a method the {section} uses for every long list.",
    "The {party} concerned in case {ref2} was told of the step in a short, polite letter signed by {name2}.",
    "Because of this, the {section2} received {k} complete files instead of a pile of loose papers.",
    "Each completed line was ticked in pencil, and {name1} initialled the last one, number {n}.",
    "The practice is simple, yet it kept the records of the {org} consistent for the whole of {month}.",
    "{name3} noted that the same approach had reduced repeated queries in the {section2} by {k} in the previous month.",
    "On this occasion the step applied to {n} entries, all of which were completed before {time2}.",
    "Any doubtful item was set aside in a separate tray marked with the date, {date}, and the initials of {name1}.",
    "This also helps a new {role} to understand how a file such as {ref} has moved through the office.",
    "{name2} added a one-line instruction to the desk manual so that the step would not be skipped on busy days.",
    "The time taken was recorded as {m} minutes, a figure {name2} keeps for planning the work of the {section}.",
    "When the {section2} asked for an update at {time2}, {name1} could answer at once by quoting reference {ref2}.",
    "Records handled this way can be found again quickly, as {name3} showed by retrieving file {ref} in under a minute.",
    "None of the {k} entries checked in this manner needed to be corrected afterwards.",
    "The step was entered in the work log for {date} together with the number of items handled, {n}.",
    "{name1} kept the original papers in their received order and placed the working notes in a separate folder.",
    "It also gave {name2} a clear record to sign, since every action under {ref2} carried a time and an initial.",
    "Of the {n} items in the morning tray, only {k} required this step, and each was marked accordingly.",
    "This mattered in case {ref}, where two papers carried almost identical numbers that differed by a single digit.",
    "The {party} who called at {time2} was pleased to hear that the request had already been recorded under {ref2}.",
    "A brief entry in the register explained the step in plain words, without any abbreviation that a reader might misread.",
    "{name3} from the {section2} confirmed the arrangement by telephone at {time2} and noted it in the reply sheet.",
    "The step added only {m} minutes to the work, while the alternative, a later search through {n} files, would have taken hours.",
    "From then on, every paper for {topic} carried the reference {ref} on its top right corner.",
];

const RECORDS: [&str; 20] = [
    "Entry {n} of the day was recorded under reference {ref}, received at {time} on {date}, with {k} pages attached.",
    "Serial {n} carried reference {ref2}; it listed {k} items, and the total was confirmed twice before the line was closed.",
    "The register showed {total} entries for {month}, of which {done} were complete and {pending} were still open.",
    "File {ref} contained {k} sheets, numbered 1 to {k}, and the number on the last sheet matched the count on the cover.",
    "Item {n} in the list, reference {ref2}, was dated {date2}, two lines below an entry dated {date}.",
    "The tally for {weekday} stood at {total} papers: {done} processed by {time2} and {pending} kept for the next day.",
    "Reference {ref} was entered as serial {n} with the date {date} and the time {time}, exactly as written on the slip.",
    "The cover sheet of {ref2} gave the count as {k} enclosures, and {name2} found the same number inside.",
    "Between {time} and {time2}, the {section} entered {total} records, an average of about {avg} each hour.",
    "The dispatch line for serial {n} read: {ref}, {k} pages, sent on {date} to the {section2}.",
    "{name1} typed reference {ref2} into the register as entry {n}, then read it back digit by digit.",
    "The monthly sheet for {month} closed at {total} items, with {pending} awaiting a reply and {done} completed.",
    "A slip attached to {ref} stated {k} enclosures; the date written on it was {date2}.",
    "In the index, serial numbers {n} to {n2} were allotted to the papers received on {date}.",
    "Reference {ref2} was received at {time2} and was the {nth} paper of the day for the {section}.",
    "The count sheet listed {total} entries in all, {done} of them cleared and {pending} on hold for documents.",
    "Line {n} of the work log recorded {ref}, the name of {name3}, and a page count of {k}.",
    "Papers numbered {n} to {n2} formed a single bundle, marked {ref} and dated {date}.",
    "The file register entry for {ref2} gave {date} as the date of receipt and {k} as the number of pages.",
    "{name2} signed the daily statement at {time2}: {total} items received, {done} disposed of, {pending} pending.",
];

const DATA_RECORDS: [&str; 12] = [
    "Line {n}: reference {ref}, amount Rs. {amount}, date {date}, status complete.",
    "Record {n2} showed code {code}-{k}-{n}, a quantity of {k} units and a value of Rs. {amount}.",
    "The figure for serial {n} was Rs. {amount}, entered against reference {ref2} on {date2}.",
    "Entry {n} listed {k} items at Rs. {unit} each, a total of Rs. {extended}.",
    "Batch {ref} held {total} records: {done} entered in full and {pending} returned for a missing field.",
    "Record {n} carried the identifier {code}/{n2}/{k}, dated {date}, with a balance of Rs. {amount}.",
    "The share of completed records was {pct} per cent, that is {done} out of {total}.",
    "For reference {ref2}, the opening figure was Rs. {amount} and the count of entries was {k}.",
    "Serial {n2}: {k} units, Rs. {unit} per unit, Rs. {extended} in total, entered at {time2}.",
    "Batch totals: {total} records keyed, {done} verified, {pending} held, as of {time2} on {date}.",
    "Line {n2} gave the date {date2}, the code {code}-{n}, and an amount of Rs. {amount}.",
    "The checker found that {done} of the {total} lines matched the source exactly, a rate of {pct} per cent.",
];

const EXCEPTIONS: [&str; 20] = [
    "One case needed more care: in file {ref}, the date on the covering note read {date2}, while the slip showed {date}.",
    "{name3} noticed that reference {ref2} had a digit missing on the forwarding slip.",
    "A paper under {ref} carried a name spelled two different ways, once on the first page and once on the last.",
    "The page count on the cover of {ref2} said {k}, but only {km1} pages were in the folder.",
    "A {party} telephoned at {time2} to say that the papers under {ref} had been sent without a signature.",
    "In entry {n}, the amount in words and the amount in figures did not agree.",
    "One slip in the bundle gave no date at all, although its reference, {ref2}, looked complete.",
    "Two papers in the tray, {ref} and {ref2}, appeared to describe the same request.",
    "The reference written on one envelope, {ref2}, did not follow the office pattern, which has four digits at the end.",
    "{name2} found that the time on one slip was written as {time2}, although the counter had closed earlier.",
    "In one form, the field for the address had been filled in with a telephone number instead.",
    "A loose sheet with no reference was found between files {ref} and {ref2}.",
    "One request, {ref}, was addressed to the {section2} but had reached the {section} by mistake.",
    "The second copy of {ref2} differed from the first in one figure on page {k}.",
    "A covering letter mentioned {k} enclosures, while the list on its reverse named only {km1}.",
    "One entry carried the date {date2}, which fell after the date on which the paper had been received.",
    "In the papers under {ref}, a page number had been written twice, so the sequence seemed to jump.",
    "The name on the application under {ref2} did not match the name on its supporting document.",
    "One file arrived with its cover torn, and part of the reference, {ref}, could not be read.",
    "A {party} asked at the counter whether request {ref2} could be treated as urgent, though no reason was given in writing.",
];

const RESOLVE: [&str; 16] = [
    "Rather than correct it from memory, {name1} marked the entry as pending and asked for written confirmation.",
    "{name1} recorded the difference in the remarks column and left the original paper exactly as received.",
    "The point was referred to {name2}, who decided that the entry should wait until the {party} replied.",
    "{name1} wrote a short note describing what was seen, without guessing which version was correct.",
    "The paper was placed in the query tray, and a letter dated {date} explained what was needed.",
    "{name2} approved a one-line remark in the register so that the next reader would know why the entry was held.",
    "Nothing was changed on the original; instead, {name1} attached a slip that set out the question in one sentence.",
    "The {section2} was informed by an internal note, and the matter was entered as pending under {ref2}.",
    "{name1} telephoned the {party} at {time2}, noted the answer word for word, and asked for it to be confirmed in writing.",
    "Following the practice of the {section}, the item was kept open until the missing detail arrived.",
    "{name3} checked the earlier record and found the correct value, which was then entered with a note of its source.",
    "The two versions were copied side by side on a query sheet, and {name2} signed it before it went out.",
    "{name1} returned the paper to the counter with a polite slip explaining the gap, dated {date}.",
    "A brief note was made, the original kept intact, and the entry flagged so that it would not be counted as complete.",
    "The query was answered the same afternoon, and {name1} completed the entry at {time2} with a reference to the reply.",
    "Instead of choosing one version, {name1} recorded both and waited for {name2} to decide.",
];

const SUMMARY: [&str; 14] = [
    "By {time2}, {total} items had been handled in all: {done} were completed and {pending} were kept pending.",
    "The closing count for the {section} was {total}, made up of {done} completed entries and {pending} that awaited a reply.",
    "At the end of {weekday}, the statement read {total} received, {done} completed, and {pending} pending.",
    "Of the {total} papers taken up on {date}, {done} were finished by {time2}; the remaining {pending} were carried forward.",
    "{name2}'s evening check confirmed {total} entries, of which {pending} were pending and {done} were closed.",
    "The day ended with {done} completed records and {pending} open ones, a total of {total}.",
    "When the register was closed at {time2}, it showed {total} entries: {done} complete and {pending} incomplete.",
    "{name1} reported a total of {total} items to {name2}, with {done} disposed of and {pending} on hold.",
    "Out of {total} entries made on {date}, only {pending} needed further action; the other {done} were complete.",
    "The tally sheet signed at {time2} recorded {done} completed items and {pending} pending items, {total} in all.",
    "Altogether, {total} papers passed through the desk, {done} of them completed and {pending} referred back.",
    "In figures, the day's work came to {total}: {done} entries complete, {pending} entries waiting.",
    "The {section} closed {weekday} with {done} finished files, {pending} pending files, and a total of {total}.",
    "The final count, checked twice, was {total} items, of which {done} were complete and {pending} were not.",
];

const CLOSERS: [&str; 18] = [
    "Handled in this way, {topic} becomes a dependable routine rather than a hurried task.",
    "The work on {topic} shows that steady attention to each detail matters more than speed alone.",
    "For {name1}, the lesson of the day was that {topic} succeeds when every step leaves a clear record.",
    "A careful record of {topic} protects the {party}, the office and the officer who must rely on it later.",
    "In the {section}, {topic} is judged by one test: could another reader follow every step without asking a question?",
    "{name2} later used the day's file as an example of how {topic} should be done.",
    "Good practice in {topic} is quiet and consistent, and it rarely draws attention to itself.",
    "The day's work on {topic} ended with no loose papers on the desk and no unexplained figures in the register.",
    "What made the work on {topic} reliable was not haste but a fixed order of checks.",
    "When {topic} is handled with care, the {party} receives a clear answer and the office keeps a clean record.",
    "Such a record of {topic} can be read years later and still make complete sense.",
    "The practice followed for {topic} at the {org} rests on accuracy, courtesy and a clear written trail.",
    "By the close of the day, {topic} had been completed in a way that {name2} could sign without hesitation.",
    "Accuracy in {topic} begins with small habits: reading back numbers, keeping originals intact, and writing plain remarks.",
    "For a {role}, {topic} is ordinary work, yet it is exactly where careful typing makes a difference.",
    "In the end, {topic} was finished on time, and every figure in it could be traced to a paper.",
    "The office manual describes {topic} in a few lines, but the day showed how much care those lines demand.",
    "Each step of {topic} had been recorded, checked and signed, leaving nothing to memory.",
];

/// Values a passage's templates draw on.
struct Ctx {
    vals: HashMap<&'static str, String>,
}

impl Ctx {
    fn fill(&self, template: &str) -> String {
        let mut out = String::with_capacity(template.len() + 64);
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            if let Some(end) = after.find('}') {
                let key = &after[..end];
                match self.vals.get(key) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push('{');
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            } else {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
        out.push_str(rest);
        // A time ending a sentence: "4:50 p.m." takes no second full stop.
        let out = out.replace("a.m..", "a.m.").replace("p.m..", "p.m.");
        fix_articles(&out)
    }
}

fn person(r: &mut Rng, taken: &mut Vec<String>) -> String {
    loop {
        let name = format!(
            "{} {}",
            FIRST[r.below(FIRST.len())],
            LAST[r.below(LAST.len())]
        );
        if !taken
            .iter()
            .any(|t| t.split(' ').next() == name.split(' ').next())
        {
            taken.push(name.clone());
            return name;
        }
    }
}

fn clock(minutes: i64) -> String {
    let (h, m) = (minutes / 60, minutes % 60);
    let (h12, half) = match h {
        0 => (12, "a.m."),
        1..=11 => (h, "a.m."),
        12 => (12, "p.m."),
        _ => (h - 12, "p.m."),
    };
    format!("{h12}:{m:02} {half}")
}

fn ordinal(n: i64) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn rupees(paise: i64) -> String {
    let whole = paise / 100;
    let s = whole.to_string();
    let mut grouped = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped}.{:02}", paise % 100)
}

fn range(r: &mut Rng, lo: i64, hi: i64) -> i64 {
    lo + i64::try_from(r.below(usize::try_from(hi - lo + 1).unwrap_or(1))).unwrap_or(0)
}

/// Lower-case the first word of a sentence when it is an ordinary word, so
/// it can follow a connective ("First, the clerk ...").
fn after_connective(fact: &str) -> String {
    // Brief facts open with an ordinary word (the briefs name no person or
    // place first); an acronym such as `EPFO` keeps its capitals.
    let first = fact.split_whitespace().next().unwrap_or("");
    let mut letters = first.chars().filter(|c| c.is_alphabetic());
    let ordinary =
        letters.next().is_some_and(char::is_uppercase) && letters.all(char::is_lowercase);
    if ordinary {
        let mut c = fact.chars();
        let head: String = c
            .next()
            .map(|h| h.to_lowercase().collect())
            .unwrap_or_default();
        format!("{head}{}", c.as_str())
    } else {
        fact.to_owned()
    }
}

/// Write one passage for `row` from its brief. `attempt` varies the seed.
#[must_use]
pub fn compose(
    row: &TitleRow,
    setting: &Setting,
    brief: &Brief,
    attempt: u64,
    global: &HashSet<String>,
) -> String {
    let seed_hex = sha256_hex(format!("{}#{attempt}", row.id).as_bytes());
    let seed = u64::from_str_radix(&seed_hex[..16], 16).unwrap_or(1);
    let mut r = Rng::new(seed);
    let data_entry = DATA_ENTRY.contains(&row.code.as_str());

    let mut taken = Vec::new();
    let name1 = person(&mut r, &mut taken);
    let name2 = person(&mut r, &mut taken);
    let name3 = person(&mut r, &mut taken);
    let year = 2026;
    let month = range(&mut r, 1, 12);
    let max_day = i64::from(radha_content::numeric::days_in_month(
        2026,
        u32::try_from(month).unwrap_or(1),
    ));
    let day = range(&mut r, 1, max_day - 3);
    let day_of = |d: i64| -> (i64, usize) {
        let mut d = d;
        // Office days only: move a Saturday or Sunday to the next Monday.
        while weekday(year, month, d) >= 5 && d < max_day {
            d += 1;
        }
        (d, weekday(year, month, d))
    };
    let (day, wd) = day_of(day);
    let (day2, _) = day_of((day + range(&mut r, 1, 3)).min(max_day));
    let date = format!(
        "{day} {} {year}",
        MONTHS[usize::try_from(month - 1).unwrap_or(0)]
    );
    let date2 = format!(
        "{day2} {} {year}",
        MONTHS[usize::try_from(month - 1).unwrap_or(0)]
    );
    let t1 = range(&mut r, 9 * 60, 11 * 60 + 45) / 5 * 5;
    let t2 = range(&mut r, 14 * 60, 17 * 60 + 30) / 5 * 5;
    let sec = &setting.sections;
    let s1 = r.below(sec.len());
    let mut s2 = r.below(sec.len());
    if s2 == s1 {
        s2 = (s1 + 1) % sec.len();
    }
    let initials = |s: &str| -> String {
        s.split_whitespace()
            .filter(|w| w.chars().next().is_some_and(char::is_uppercase))
            .filter_map(|w| w.chars().next())
            .take(2)
            .collect()
    };
    let refno = |r: &mut Rng, s: &str| {
        format!(
            "{}/{}/{year}/{:04}",
            setting.code,
            initials(s),
            range(r, 101, 9899)
        )
    };
    let ref1 = refno(&mut r, &sec[s1]);
    let ref2 = refno(&mut r, &sec[s2]);
    let total = range(&mut r, 24, 96);
    let pending = range(&mut r, 2, total / 4);
    let done = total - pending;
    let n = range(&mut r, 11, 60);
    let k = range(&mut r, 4, 19);
    let units = range(&mut r, 3, 48);
    let unit_paise = range(&mut r, 1_200, 98_000) / 5 * 5;
    let hours = ((t2 - t1) / 60).max(1);
    // Exact when it is a whole number, otherwise rounded and said so.
    let pct = if done * 100 % total == 0 {
        format!("{}", done * 100 / total)
    } else {
        format!("about {}", (done * 100 + total / 2) / total)
    };
    let mut vals: HashMap<&'static str, String> = HashMap::new();
    vals.insert("name1", name1);
    vals.insert("name2", name2);
    vals.insert("name3", name3);
    vals.insert("role", setting.role.clone());
    vals.insert("org", setting.org.clone());
    vals.insert("place", setting.place.clone());
    vals.insert("section", sec[s1].clone());
    vals.insert("section2", sec[s2].clone());
    vals.insert("party", setting.party.clone());
    vals.insert("code", setting.code.clone());
    vals.insert("topic", brief.topic.clone());
    vals.insert("title", row.title.clone());
    vals.insert("date", date);
    vals.insert("date2", date2);
    vals.insert("weekday", WEEKDAYS[wd].to_owned());
    vals.insert(
        "month",
        MONTHS[usize::try_from(month - 1).unwrap_or(0)].to_owned(),
    );
    vals.insert("time", clock(t1));
    vals.insert("time2", clock(t2));
    vals.insert("ref", ref1);
    vals.insert("ref2", ref2);
    vals.insert("total", total.to_string());
    vals.insert("done", done.to_string());
    vals.insert("pending", pending.to_string());
    vals.insert("n", n.to_string());
    vals.insert("n2", (n + k).to_string());
    vals.insert("nth", ordinal(range(&mut r, 2, 30)));
    vals.insert("k", k.to_string());
    vals.insert("km1", (k - 1).to_string());
    vals.insert("m", range(&mut r, 6, 25).to_string());
    vals.insert("avg", (total / hours).to_string());
    vals.insert("amount", rupees(range(&mut r, 50_000, 9_999_999) / 5 * 5));
    vals.insert("unit", rupees(unit_paise));
    vals.insert("extended", rupees(unit_paise * units));
    vals.insert("pct", pct);
    // `k` in the unit records is the unit count.
    let ctx = Ctx { vals };
    let unit_ctx = {
        let mut v = ctx.vals.clone();
        v.insert("k", units.to_string());
        Ctx { vals: v }
    };

    // Pick a template of `pool` not used yet in this passage whose filled
    // sentence no earlier passage of the pool holds. `None` when every
    // template is used up or taken.
    let mut used: HashSet<usize> = HashSet::new();
    let mut take = |r: &mut Rng, pool: &[&str], salt: usize, c: &Ctx| -> Option<String> {
        let start = r.below(pool.len());
        for k in 0..pool.len() {
            let i = (start + k) % pool.len();
            if used.contains(&(salt * 1000 + i)) {
                continue;
            }
            let s = c.fill(pool[i]);
            if !global.contains(&s) {
                used.insert(salt * 1000 + i);
                return Some(s);
            }
        }
        None
    };
    let mut conn = CONNECT.to_vec();
    let mut connective = |r: &mut Rng| -> &'static str {
        if conn.is_empty() {
            return "";
        }
        let i = r.below(conn.len());
        conn.remove(i)
    };
    let fact =
        |r: &mut Rng, i: usize, connective: &mut dyn FnMut(&mut Rng) -> &'static str| -> String {
            let f = ctx.fill(&brief.facts[i]);
            if r.below(3) == 0 {
                f
            } else {
                format!("{}{}", connective(r), after_connective(&f))
            }
        };

    let mut p1 = Vec::new();
    p1.extend(take(&mut r, &OPENERS, 1, &ctx));
    p1.extend(take(&mut r, &CONTEXTS, 2, &ctx));
    let mut p2 = vec![fact(&mut r, 0, &mut connective)];
    p2.extend(take(&mut r, &ELABORATE, 3, &ctx));
    p2.push(fact(&mut r, 1, &mut connective));
    p2.extend(take(&mut r, &ELABORATE, 3, &ctx));
    let mut p3 = Vec::new();
    let records = if data_entry { 5 } else { 2 };
    for _ in 0..records {
        let line = if data_entry && r.below(3) != 0 {
            take(&mut r, &DATA_RECORDS, 4, &unit_ctx)
        } else {
            take(&mut r, &RECORDS, 5, &ctx)
        };
        p3.extend(line);
    }
    p3.push(fact(&mut r, 2, &mut connective));
    p3.extend(take(&mut r, &EXCEPTIONS, 6, &ctx));
    p3.extend(take(&mut r, &RESOLVE, 7, &ctx));
    let mut p4 = vec![fact(&mut r, 3, &mut connective)];
    p4.extend(take(&mut r, &ELABORATE, 3, &ctx));
    // Facts beyond four are extra material for long passages.
    for i in 4..brief.facts.len() {
        p4.push(fact(&mut r, i, &mut connective));
    }
    p4.extend(take(&mut r, &SUMMARY, 8, &ctx));
    p4.extend(take(&mut r, &CLOSERS, 9, &ctx));

    let words = |ps: &[&Vec<String>]| -> usize {
        ps.iter()
            .flat_map(|p| p.iter())
            .map(|s| s.split_whitespace().count())
            .sum()
    };
    // Grow to the minimum with further, non-repeating material.
    let mut extra = 0;
    while words(&[&p1, &p2, &p3, &p4]) < MIN_WORDS + 15 && extra < 24 {
        let s = match extra % 3 {
            1 => take(&mut r, &RECORDS, 5, &ctx),
            _ => take(&mut r, &ELABORATE, 3, &ctx),
        };
        if let Some(s) = s {
            match extra % 3 {
                0 => p2.push(s),
                1 => p3.insert(0, s),
                _ => p4.insert(1, s),
            }
        }
        extra += 1;
    }
    if data_entry {
        // Data-entry tests are about key depressions: keep adding records.
        let mut guard = 0;
        while [&p1, &p2, &p3, &p4]
            .iter()
            .flat_map(|p| p.iter())
            .map(String::len)
            .sum::<usize>()
            < MIN_KEYS_DATA_ENTRY + 400
            && guard < 8
        {
            p3.extend(take(&mut r, &DATA_RECORDS, 4, &unit_ctx));
            guard += 1;
        }
    }
    let text = [p1, p2, p3, p4]
        .iter()
        .map(|p| p.join(" "))
        .collect::<Vec<_>>()
        .join("\n");
    fix_articles(&text)
}

/// `a` before a vowel sound becomes `an` (roles and parties are filled in
/// after the article was written).
#[must_use]
pub fn fix_articles(text: &str) -> String {
    let keep_a = [
        "one", "once", "euro", "unit", "user", "uniform", "union", "unique", "usual",
    ];
    let mut out = String::with_capacity(text.len() + 16);
    let mut words = text.split(' ').peekable();
    let mut first = true;
    while let Some(w) = words.next() {
        if !first {
            out.push(' ');
        }
        first = false;
        let next = words.peek().copied().unwrap_or("");
        let lower = next.to_lowercase();
        let vowel =
            lower.starts_with(['a', 'e', 'i', 'o']) && !keep_a.iter().any(|k| lower.starts_with(k));
        match w {
            "a" if vowel => out.push_str("an"),
            "A" if vowel => out.push_str("An"),
            "\na" if vowel => out.push_str("\nan"),
            _ => out.push_str(w),
        }
    }
    out
}

/// Build the 500 passages in title-bank order.
///
/// # Errors
///
/// When a written ID has no brief or profile setting, a passage misses its
/// length rules, or no seed keeps the pool free of shared sentences.
pub fn build(
    rows: &[TitleRow],
    drafts: &Drafts,
    settings: &HashMap<String, Setting>,
    briefs: &HashMap<String, Brief>,
    briefs_sha: &str,
) -> anyhow::Result<(Vec<ImportedExercise>, usize, usize)> {
    let mut used: HashSet<String> = HashSet::new();
    let note_sentences = |text: &str, used: &mut HashSet<String>| -> bool {
        let mine: Vec<String> = quality::sentences(text)
            .into_iter()
            .filter(|s| s.split_whitespace().count() >= 6)
            .collect();
        let local: HashSet<&String> = mine.iter().collect();
        if local.len() != mine.len() || mine.iter().any(|s| used.contains(s)) {
            return false;
        }
        used.extend(mine);
        true
    };
    // Reused drafts claim their sentences first.
    for row in rows {
        if let Some((text, _, _)) = drafts.get(&row.id) {
            let _ = note_sentences(text, &mut used);
        }
    }
    let mut out = Vec::new();
    let (mut reused, mut written) = (0, 0);
    for (i, row) in rows.iter().enumerate() {
        let number = u32::try_from(i + 1).context("count")?;
        let (content, status, path, sha) = if let Some((text, path, sha)) = drafts.get(&row.id) {
            reused += 1;
            (
                text.clone(),
                ContentStatus::Provisional,
                path.clone(),
                sha.clone(),
            )
        } else {
            let setting = settings
                .get(&row.code)
                .with_context(|| format!("no profile setting for {}", row.code))?;
            let brief = briefs
                .get(&row.id)
                .with_context(|| format!("no brief for {} ({})", row.id, row.title))?;
            let mut text = None;
            for attempt in 0..40 {
                let t = compose(row, setting, brief, attempt, &used);
                if t.contains('{') {
                    bail!("{}: unfilled placeholder in {t:?}", row.id);
                }
                if note_sentences(&t, &mut used) {
                    text = Some(t);
                    break;
                }
            }
            let text =
                text.with_context(|| format!("{}: no seed avoids shared sentences", row.id))?;
            written += 1;
            (
                text,
                ContentStatus::GeneratedReviewPending,
                format!("radha-exam-briefs:data/exam-english-briefs.txt#{}", row.id),
                briefs_sha.to_owned(),
            )
        };
        let words = content.split_whitespace().count();
        if words < MIN_WORDS {
            bail!("{} has {words} words (at least {MIN_WORDS})", row.id);
        }
        if DATA_ENTRY.contains(&row.code.as_str()) && content.chars().count() < MIN_KEYS_DATA_ENTRY
        {
            bail!(
                "{} has fewer than {MIN_KEYS_DATA_ENTRY} key depressions",
                row.id
            );
        }
        let mut e = ImportedExercise {
            number,
            title: row.title.clone(),
            category: Some(row.profile.clone()),
            difficulty: Some(3),
            content,
            source_id: row.id.clone(),
            status,
            source_path: path,
            source_sha256: sha,
            ..ImportedExercise::default()
        };
        e.seal();
        out.push(e);
    }
    Ok((out, reused, written))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekdays_are_real() {
        assert_eq!(WEEKDAYS[weekday(2026, 1, 1)], "Thursday");
        assert_eq!(WEEKDAYS[weekday(2026, 4, 15)], "Wednesday");
        assert_eq!(WEEKDAYS[weekday(2024, 2, 29)], "Thursday");
    }

    #[test]
    fn helpers() {
        assert_eq!(clock(9 * 60 + 5), "9:05 a.m.");
        assert_eq!(clock(13 * 60 + 30), "1:30 p.m.");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(22), "22nd");
        assert_eq!(rupees(1_234_550), "12,345.50");
        assert_eq!(after_connective("The clerk wrote."), "the clerk wrote.");
        assert_eq!(
            after_connective("A petition is filed."),
            "a petition is filed."
        );
        assert_eq!(
            after_connective("Invoices are entered."),
            "invoices are entered."
        );
        assert_eq!(after_connective("EPFO staff wrote."), "EPFO staff wrote.");
        assert_eq!(
            fix_articles("a officer and a unit"),
            "an officer and a unit"
        );
    }
}

//! Track 03 · Number + Punctuation: Golu's 50 seed drills (ALN-001 …
//! ALN-050, `data/track03-alphanumeric-50-seed.md`) expanded to full drills.
//!
//! Every drill keeps its own seed lines (typed to US-keyboard ASCII) and is
//! extended by a generator written for its title until it holds at least
//! [`MIN_WORDS`] space-separated tokens. No line appears twice in the whole
//! track. Every calculation is built from integer cents and then re-checked
//! by `radha_content::numeric`, so a wrong sum, an impossible date or an
//! invalid time aborts the import.
//!
//! Characters a US keyboard has no key for are written the ASCII way, and
//! each affected drill says so in its first line: `Rs.` for the rupee sign,
//! `EUR` for the euro sign, `*` for multiplication, `/` for division, `-`
//! for an en dash and `--` for an em dash. An approximate result (`≈`) is
//! written as `=` under the track's rounding rule (two decimals, half away
//! from zero).

use std::collections::HashSet;

use anyhow::{bail, Context};
use radha_content::codec::sha256_hex;
use radha_content::import::ImportedExercise;
use radha_content::numeric;
use radha_content::ContentStatus;

use crate::rng::Rng;

/// Minimum space-separated tokens per drill (Golu: "2000+ words").
pub const MIN_WORDS: usize = 2000;

/// Pipeline label (bundle `source`).
pub const PIPELINE: &str = "radha-track03:data/track03-alphanumeric-50-seed.md + seeded expansion";

/// One seed drill as Golu supplied it.
#[derive(Debug, Clone)]
pub struct Seed {
    /// `ALN-001`.
    pub id: String,
    /// Title.
    pub title: String,
    /// Category line.
    pub category: String,
    /// The seed lines, as written.
    pub lines: Vec<String>,
}

/// What the build changed, for the report.
#[derive(Debug, Default)]
pub struct Notes {
    /// Characters mapped to ASCII, with counts.
    pub mapped: Vec<(char, usize)>,
    /// Seed lines corrected (`id: old -> new`).
    pub fixed: Vec<String>,
    /// Seed lines dropped because an earlier drill already has them.
    pub repeated_seed_lines: usize,
}

/// Parse the seed markdown: `## ALN-NNN — Title`, `Category: …`, then one
/// fenced block of drill lines.
///
/// # Errors
///
/// When a section has no id, title or lines.
pub fn parse_seeds(md: &str) -> anyhow::Result<Vec<Seed>> {
    let mut seeds: Vec<Seed> = Vec::new();
    let mut in_block = false;
    for line in md.lines() {
        let trimmed = line.trim_end();
        if let Some(head) = trimmed.strip_prefix("## ") {
            let (id, title) = head
                .split_once(" — ")
                .with_context(|| format!("seed heading without a title: {head:?}"))?;
            seeds.push(Seed {
                id: id.trim().to_owned(),
                title: title.trim().to_owned(),
                category: String::new(),
                lines: Vec::new(),
            });
            in_block = false;
            continue;
        }
        let Some(seed) = seeds.last_mut() else {
            continue;
        };
        if let Some(cat) = trimmed.strip_prefix("Category:") {
            seed.category = cat.trim().to_owned();
        } else if trimmed.starts_with("```") {
            in_block = !in_block;
        } else if in_block && !trimmed.trim().is_empty() {
            seed.lines.push(trimmed.to_owned());
        }
    }
    for (i, s) in seeds.iter().enumerate() {
        let expected = format!("ALN-{:03}", i + 1);
        if s.id != expected || s.title.is_empty() || s.lines.is_empty() || s.category.is_empty() {
            bail!(
                "seed {} is incomplete or out of order (expected {expected})",
                s.id
            );
        }
    }
    Ok(seeds)
}

/// Type `line` the US-keyboard way (module docs), collapsing runs of spaces
/// (a double space would be scored as an extra keystroke).
pub fn ascii_line(line: &str, mapped: &mut Vec<(char, usize)>) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    for c in line.chars() {
        let rep: Option<&str> = match c {
            '₹' => Some("Rs."),
            '€' => Some("EUR "),
            '×' => Some("*"),
            '÷' => Some("/"),
            '≈' => Some("="),
            '–' => Some("-"),
            '—' => Some("--"),
            _ => None,
        };
        match rep {
            Some(r) => {
                match mapped.iter_mut().find(|(m, _)| *m == c) {
                    Some((_, n)) => *n += 1,
                    None => mapped.push((c, 1)),
                }
                out.push_str(r);
            }
            None => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Seed lines that are wrong as written, with their corrections: an
/// impossible clock time, a date with month 17, a ledger total that skips
/// one entry (so its closing balance disagreed with the ledger above it).
fn seed_fix(id: &str, line: &str) -> Option<&'static str> {
    Some(match (id, line) {
        ("ALN-022", "123:45? 67:89? 90:12?") => "123:45? 06:39? 09:12?",
        ("ALN-048", "07/11/2026 07/17/2026 17/11/2026") => "07/11/2026 17/07/2026 17/11/2026",
        ("ALN-036", "1250.00 + 2200.00 + 500.00 + 1750.00 = 5700.00") => {
            "1250.00 + 2200.00 + 500.00 + 1750.00 + 250.00 = 5950.00"
        }
        ("ALN-036", "5700.00 - 3450.00 = 2250.00") => "5950.00 - 3450.00 = 2500.00",
        _ => return None,
    })
}

/// The first line of a drill whose seed used a character without a US key,
/// or whose sums round: how this track writes it. Unique per drill.
fn key_note(id: &str) -> Option<&'static str> {
    Some(match id {
        "ALN-009" => "Key note: the rupee sign is typed Rs. before the amount, as in Rs.125.00.",
        "ALN-010" => "Checkpoint key note: write rupees as Rs. and keep every comma and decimal point.",
        "ALN-018" => "Percent rule: a result is rounded to two decimals, half away from zero.",
        "ALN-019" => "Range note: an en dash is typed as one hyphen (-) and an em dash as two (--).",
        "ALN-022" => "Quote drill note: amounts in rupees are typed with Rs. in front.",
        "ALN-026" => "Dash rule: hyphen and en dash are both typed -, an em dash is typed --.",
        "ALN-028" => "Currency rule: write the rupee as Rs., the dollar as $ and the euro as EUR; multiply with * and divide with /.",
        "ALN-029" => "Mixed note: Rs. stands for the rupee sign in every amount below.",
        "ALN-030" => "Assessment note: Rs. is the rupee sign; - is the en dash and -- the em dash.",
        "ALN-031" => "Cash rule: multiply with *, and every total keeps two decimals.",
        "ALN-033" => "Invoice rule: the tax is the amount times the rate, rounded to two decimals, half away from zero.",
        "ALN-034" => "Interest rule: the interest is the principal times the rate, rounded to two decimals, half up.",
        "ALN-038" => "Verification note: Rs. is the rupee sign and * is the multiplication sign.",
        "ALN-044" => "Notice note: amounts carry Rs. and time bands use a plain hyphen.",
        "ALN-045" => "Record note: Rs. marks rupee amounts inside the pipe-separated fields.",
        "ALN-046" => "Clause note: Rs. is the rupee sign; -- and - stand for em and en dashes.",
        "ALN-047" => "Alternation note: rupee amounts are typed with Rs. on both sides of the bar.",
        "ALN-048" => "Recovery note: Rs. replaces the rupee sign; -- replaces the em dash.",
        "ALN-049" => "Endurance note: rupee amounts are written Rs. followed by the figure.",
        "ALN-050" => "Final note: Rs. for rupees, * to multiply, two decimals for money, half up.",
        _ => return None,
    })
}

// --- small formatting helpers ------------------------------------------------------

fn pick<'a>(r: &mut Rng, items: &[&'a str]) -> &'a str {
    items[r.below(items.len())]
}

fn range(r: &mut Rng, lo: i64, hi: i64) -> i64 {
    lo + i64::try_from(r.below(usize::try_from(hi - lo + 1).unwrap_or(1))).unwrap_or(0)
}

fn digits_from(r: &mut Rng, set: &[u8], len: usize) -> String {
    (0..len)
        .map(|_| char::from(set[r.below(set.len())]))
        .collect()
}

fn any_digits(r: &mut Rng, len: usize) -> String {
    digits_from(r, b"0123456789", len)
}

/// `lo` to `lo + spread - 1` random digits.
fn some_digits(r: &mut Rng, lo: usize, spread: usize) -> String {
    let len = lo + r.below(spread);
    any_digits(r, len)
}

/// `1250.50` from cents.
fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let c = cents.abs();
    format!("{sign}{}.{:02}", c / 100, c % 100)
}

/// `1,250.50` from cents (Western thousands grouping).
fn money_commas(cents: i64) -> String {
    let plain = money(cents.abs());
    let (whole, frac) = plain.split_once('.').unwrap_or((&plain, "00"));
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{}{grouped}.{frac}", if cents < 0 { "-" } else { "" })
}

fn cents(r: &mut Rng, lo: i64, hi: i64, step: i64) -> i64 {
    range(r, lo / step, hi / step) * step
}

/// A real calendar date in 2024–2028.
fn date(r: &mut Rng) -> (i64, i64, i64) {
    let y = range(r, 2024, 2028);
    let m = range(r, 1, 12);
    let max = i64::from(numeric::days_in_month(
        u32::try_from(y).unwrap_or(2026),
        u32::try_from(m).unwrap_or(1),
    ));
    (y, m, range(r, 1, max))
}

fn dmy(d: (i64, i64, i64), sep: char) -> String {
    format!("{:02}{sep}{:02}{sep}{}", d.2, d.1, d.0)
}

fn ymd(d: (i64, i64, i64), sep: char) -> String {
    format!("{}{sep}{:02}{sep}{:02}", d.0, d.1, d.2)
}

fn time(r: &mut Rng) -> (i64, i64) {
    (range(r, 0, 23), range(r, 0, 59))
}

fn hm(t: (i64, i64)) -> String {
    format!("{:02}:{:02}", t.0, t.1)
}

fn hms(r: &mut Rng) -> String {
    let t = time(r);
    format!("{}:{:02}", hm(t), range(r, 0, 59))
}

/// Two times of one day, the first earlier.
fn time_band(r: &mut Rng) -> String {
    let a = range(r, 0, 22 * 60);
    let b = range(r, a + 5, (a + 240).min(23 * 60 + 59));
    format!("{}-{}", hm((a / 60, a % 60)), hm((b / 60, b % 60)))
}

fn join(parts: Vec<String>) -> String {
    parts.join(" ")
}

fn n_of(r: &mut Rng, lo: usize, hi: usize, mut f: impl FnMut(&mut Rng) -> String) -> String {
    let n = lo + r.below(hi - lo + 1);
    join((0..n).map(|_| f(r)).collect())
}

// --- arithmetic lines (built from cents, so they are right by construction) --------

fn sum_line(r: &mut Rng, cur: &str, terms: usize) -> String {
    let vals: Vec<i64> = (0..terms).map(|_| cents(r, 500, 999_999, 5)).collect();
    let total: i64 = vals.iter().sum();
    let lhs: Vec<String> = vals.iter().map(|v| format!("{cur}{}", money(*v))).collect();
    format!("{} = {cur}{}", lhs.join(" + "), money(total))
}

fn diff_line(r: &mut Rng, cur: &str) -> String {
    let a = cents(r, 10_000, 2_500_000, 5);
    let b = cents(r, 100, a, 5);
    format!(
        "{cur}{} - {cur}{} = {cur}{}",
        money(a),
        money(b),
        money(a - b)
    )
}

fn product_line(r: &mut Rng, cur: &str) -> String {
    let n = range(r, 2, 24);
    let p = cents(r, 100, 250_000, 25);
    format!("{n} * {cur}{} = {cur}{}", money(p), money(p * n))
}

fn division_line(r: &mut Rng, cur: &str) -> String {
    let n = range(r, 2, 12);
    let each = cents(r, 500, 150_000, 5);
    format!("{cur}{} / {n} = {cur}{}", money(each * n), money(each))
}

/// `a * r% = t` with `t` rounded half up (cents), returning (text, t).
fn rate_of(a: i64, rate_tenths: i64) -> i64 {
    // a cents * rate% = a * rate_tenths / 1000 cents, rounded half up.
    let num = a * rate_tenths;
    (num + 500).div_euclid(1000)
}

fn rate_text(rate_tenths: i64) -> String {
    if rate_tenths % 10 == 0 {
        format!("{}%", rate_tenths / 10)
    } else {
        format!("{}.{}%", rate_tenths / 10, rate_tenths % 10)
    }
}

fn tax_line(r: &mut Rng) -> String {
    let a = cents(r, 10_000, 9_999_999, 1);
    let rate = [50, 120, 180, 280, 30, 250][r.below(6)];
    let t = rate_of(a, rate);
    format!(
        "{} * {} = {}; {} + {} = {}",
        money(a),
        rate_text(rate),
        money(t),
        money(a),
        money(t),
        money(a + t)
    )
}

fn interest_line(r: &mut Rng) -> String {
    let p = range(r, 4, 400) * 50_000; // 2000.00 .. 200000.00
    let rate = [25, 30, 35, 40, 45, 50, 55, 60, 65, 70, 75, 80, 85][r.below(13)];
    let i = rate_of(p, rate);
    if r.below(2) == 0 {
        format!("{} * {} = {}", money(p), rate_text(rate), money(i))
    } else {
        format!(
            "{} * {} = {}; {} + {} = {}",
            money(p),
            rate_text(rate),
            money(i),
            money(p),
            money(i),
            money(p + i)
        )
    }
}

fn percent_line(r: &mut Rng) -> String {
    let rates = [
        10, 20, 25, 40, 50, 75, 80, 100, 120, 125, 150, 200, 250, 300, 400, 500,
    ];
    loop {
        let p = rates[r.below(rates.len())]; // tenths of a percent
        let a = range(r, 1, 250) * 20;
        let up = a * (1000 + p);
        let down = a * (1000 - p);
        let line = match r.below(4) {
            0 if up % 10 == 0 => format!("{a} + {} = {}", rate_text(p), fmt_tenths(up / 10)),
            1 if down % 10 == 0 => format!("{a} - {} = {}", rate_text(p), fmt_tenths(down / 10)),
            2 if (a * p) % 10 == 0 => {
                format!("{a} * {} = {}", rate_text(p), fmt_tenths(a * p / 10))
            }
            3 => {
                let whole = [
                    20, 25, 40, 50, 60, 75, 80, 120, 150, 160, 200, 240, 300, 400, 500,
                ];
                let y = whole[r.below(whole.len())];
                let x = range(r, 1, y);
                if (x * 100) % y != 0 {
                    continue;
                }
                format!("{x}/{y} = {}%", x * 100 / y)
            }
            _ => continue,
        };
        return line;
    }
}

/// A value held in hundredths (`a * (1000 + p) / 10` units of 1/100).
fn fmt_tenths(hundredths: i64) -> String {
    if hundredths % 100 == 0 {
        format!("{}", hundredths / 100)
    } else {
        money(hundredths)
    }
}

// --- the drill generators ----------------------------------------------------------

/// One new line for drill `n` (1-based).
#[allow(clippy::too_many_lines)]
fn line_for(n: usize, r: &mut Rng) -> String {
    match n {
        1 => n_of(r, 6, 9, |r| {
            let len = 1 + r.below(5);
            digits_from(r, b"12345", len)
        }),
        2 => n_of(r, 6, 9, |r| {
            let len = 1 + r.below(5);
            digits_from(r, b"67890", len)
        }),
        3 => n_of(r, 5, 8, |r| {
            let start = r.below(10);
            let len = 3 + r.below(6);
            (0..len)
                .map(|k| char::from(b'0' + u8::try_from((start + k) % 10).unwrap_or(0)))
                .collect()
        }),
        4 => n_of(r, 5, 8, |r| {
            let start = r.below(10) + 10;
            let len = 3 + r.below(6);
            (0..len)
                .map(|k| char::from(b'0' + u8::try_from((start - k) % 10).unwrap_or(0)))
                .collect()
        }),
        5 => n_of(r, 5, 8, |r| {
            let len = 4 + r.below(5);
            let left_first = r.below(2) == 0;
            (0..len)
                .map(|k| {
                    let left = (k % 2 == 0) == left_first;
                    let set: &[u8] = if left { b"12345" } else { b"67890" };
                    char::from(set[r.below(5)])
                })
                .collect()
        }),
        6 => n_of(r, 6, 9, |r| {
            let a = char::from(b'0' + u8::try_from(r.below(10)).unwrap_or(0));
            let mut b = a;
            while b == a {
                b = char::from(b'0' + u8::try_from(r.below(10)).unwrap_or(0));
            }
            let pat = pick(
                r,
                &[
                    "aa", "aaa", "aaaa", "aabb", "abab", "abba", "aaabbb", "abaaba", "aabbaa",
                    "baab", "abbbba", "aaaaaa",
                ],
            );
            pat.chars().map(|c| if c == 'a' { a } else { b }).collect()
        }),
        7 => {
            let three = r.below(2) == 0;
            let (lo, hi) = if three { (100, 999) } else { (10, 99) };
            let step =
                range(r, 1, if three { 97 } else { 9 }) * if r.below(2) == 0 { 1 } else { -1 };
            let mut v = range(r, lo, hi);
            let mut out = Vec::new();
            while out.len() < 12 && (lo..=hi).contains(&v) {
                out.push(v.to_string());
                v += step;
            }
            if out.len() < 4 {
                out.push(range(r, lo, hi).to_string());
            }
            join(out)
        }
        8 => n_of(r, 5, 8, |r| match r.below(4) {
            0 => digits_from(r, b"789456123", 3),
            1 => {
                let cols = [
                    "147", "258", "369", "741", "852", "963", "789", "456", "123", "987", "654",
                    "321",
                ];
                format!("{}{}", pick(r, &cols), pick(r, &cols))
            }
            2 => format!("{}.{}", range(r, 1, 999), any_digits(r, 2)),
            _ => format!("{}0", digits_from(r, b"789456123", 2)),
        }),
        9 | 47 => {
            let item = |r: &mut Rng| match r.below(8) {
                0 => some_digits(r, 6, 5),
                1 => money(cents(r, 100, 999_999, 1)),
                2 => dmy(date(r), '/'),
                3 => hm(time(r)),
                4 => format!(
                    "{}%",
                    pick(
                        r,
                        &["5", "7.5", "10", "12.5", "15", "25", "50", "75", "100"]
                    )
                ),
                5 => format!("Rs.{}", money(cents(r, 100, 999_999, 5))),
                6 => format!(
                    "({}) [{}] {{{}}}",
                    any_digits(r, 3),
                    any_digits(r, 3),
                    any_digits(r, 3)
                ),
                _ => format!(
                    "{}-{}-{}",
                    any_digits(r, 2),
                    any_digits(r, 2),
                    any_digits(r, 2)
                ),
            };
            let k = 1 + r.below(3);
            let side: Vec<String> = (0..k).map(|_| item(r)).collect();
            let side = side.join(" ");
            if n == 47 && r.below(2) == 0 {
                let other = item(r);
                format!("{side} | {side} | {other} | {other}")
            } else {
                format!("{side} | {side}")
            }
        }
        10 => {
            let k = [1, 2, 3, 4, 5, 6, 7, 8, 11, 13, 14, 15, 16, 18, 19][r.below(15)];
            line_for(k, r)
        }
        11 => {
            let base = format!("{}{}", range(r, 1, 9), any_digits(r, 11));
            let mut out = vec![base.clone()];
            while out.len() < 3 {
                let mut b: Vec<u8> = base.clone().into_bytes();
                let at = 1 + r.below(10);
                b.swap(at, at + 1);
                let v = String::from_utf8(b).unwrap_or_default();
                if !out.contains(&v) {
                    out.push(v);
                }
            }
            join(out)
        }
        12 | 40 => {
            let y = range(r, 2024, 2028);
            let start = range(r, 1, 9000);
            let count = 3 + r.below(2);
            let fmt = r.below(10);
            let a = range(r, 1, 99);
            let b = range(r, 100, 999);
            join(
                (0..count)
                    .map(|k| {
                        let s = start + i64::try_from(k).unwrap_or(0);
                        match fmt {
                            0 => format!("{s:04}-{y}"),
                            1 => format!("{a:02}{:02}/{s:04}", y % 100),
                            2 => format!("{s:06}"),
                            3 => format!("{b}-{a:03}-{s:03}"),
                            4 => format!("{y}-{s:05}"),
                            // Five-digit serial: never mistaken for a date.
                            5 => format!("{a:02}/{:02}/{s:05}", y % 100),
                            6 => format!("{y}.{b}.{s:04}"),
                            7 => format!("{b}/{y}/{s:03}"),
                            8 => format!("{:02}-{b}-{s:04}", a % 100),
                            _ => format!("{s:04}_{y}"),
                        }
                    })
                    .collect(),
            )
        }
        13 => {
            let fmt = r.below(5);
            n_of(r, 4, 5, |r| {
                let d = date(r);
                match fmt {
                    0 => dmy(d, '/'),
                    1 => dmy(d, '-'),
                    2 => dmy(d, '.'),
                    3 => ymd(d, '-'),
                    _ => ymd(d, '/'),
                }
            })
        }
        14 => n_of(
            r,
            5,
            7,
            |r| if r.below(4) == 0 { hms(r) } else { hm(time(r)) },
        ),
        15 => n_of(r, 6, 8, |r| {
            let places = 1 + r.below(4);
            format!("{}.{}", range(r, 0, 9999), any_digits(r, places))
        }),
        16 => n_of(r, 6, 10, |r| {
            let sign = if r.below(2) == 0 { "-" } else { "+" };
            match r.below(3) {
                0 => format!("{sign}{}", range(r, 1, 9999)),
                1 => format!("{sign}{}", money(cents(r, 1, 999_999, 1))),
                _ => format!("{sign}{}.{}", range(r, 0, 99), any_digits(r, 1)),
            }
        }),
        17 => match r.below(5) {
            0 => n_of(r, 6, 9, |r| {
                format!("{}:{}", range(r, 1, 12), range(r, 1, 16))
            }),
            1 => {
                let (a, b) = (range(r, 1, 9), range(r, 2, 11));
                let k = range(r, 2, 6);
                let m = range(r, k + 1, 12);
                format!("{a}:{b} = {}:{} = {}:{}", a * k, b * k, a * m, b * m)
            }
            2 => {
                let (a, b) = (range(r, 1, 11), range(r, 2, 12));
                let k = range(r, 2, 5);
                let m = range(r, k + 1, 9);
                format!("{a}/{b} = {}/{} = {}/{}", a * k, b * k, a * m, b * m)
            }
            3 => {
                let den = [2, 4, 5, 8, 10, 16, 20, 25, 40, 50][r.below(10)];
                let num = range(r, 1, den - 1);
                let value = num * 10_000 / den; // exact: den divides 10000
                let mut text = format!("{}.{:04}", value / 10_000, value % 10_000);
                while text.ends_with('0') {
                    text.pop();
                }
                format!("{num}/{den} = {text}")
            }
            _ => n_of(r, 6, 9, |r| {
                let den = range(r, 2, 32);
                format!("{}/{den}", range(r, 1, den - 1))
            }),
        },
        18 => {
            if r.below(4) == 0 {
                n_of(r, 6, 9, |r| {
                    format!(
                        "{}%",
                        pick(
                            r,
                            &[
                                "1", "2.5", "5", "7.5", "10", "12.5", "15", "17.5", "20", "22.5",
                                "25", "30", "33", "37.5", "40", "45", "50", "62.5", "75", "87.5",
                                "90", "100"
                            ]
                        )
                    )
                })
            } else {
                percent_line(r)
            }
        }
        19 => match r.below(5) {
            0 => n_of(r, 4, 6, |r| {
                let a = range(r, 1, 9000);
                format!("{a}-{}", a + range(r, 9, 999))
            }),
            1 => {
                let d1 = date(r);
                let d2 = (
                    d1.0,
                    d1.1,
                    range(
                        r,
                        d1.2,
                        i64::from(numeric::days_in_month(
                            u32::try_from(d1.0).unwrap_or(2026),
                            u32::try_from(d1.1).unwrap_or(1),
                        )),
                    ),
                );
                format!("{}-{}", dmy(d1, '/'), dmy(d2, '/'))
            }
            2 => n_of(r, 3, 4, time_band),
            3 => {
                let a = range(r, 1, 300);
                let s = range(r, 5, 50);
                format!(
                    "{a}-{}-{} {}-{}-{}",
                    a + s,
                    a + 2 * s,
                    a + 3 * s,
                    a + 4 * s,
                    a + 5 * s
                )
            }
            _ => n_of(r, 3, 5, |r| {
                let a = range(r, 1, 400) * 100;
                format!(
                    "{}-{}",
                    money_commas(a * 100).trim_end_matches(".00"),
                    money_commas((a + range(r, 1, 50) * 100) * 100).trim_end_matches(".00")
                )
            }),
        },
        20 | 48 => {
            let base = some_digits(r, 5, 4);
            let mut out = vec![base.clone()];
            while out.len() < 5 {
                let mut b: Vec<u8> = base.clone().into_bytes();
                let at = r.below(b.len() - 1);
                match r.below(3) {
                    0 => b.swap(at, at + 1),
                    1 => b[at] = b'0' + u8::try_from(r.below(10)).unwrap_or(0),
                    _ => {
                        b.insert(at + 1, b'.');
                    }
                }
                let v = String::from_utf8(b).unwrap_or_default();
                if !out.contains(&v) && !v.ends_with('.') {
                    out.push(v);
                }
            }
            join(out)
        }
        21 => {
            let k = 4 + r.below(4);
            let mut s = String::new();
            for i in 0..k {
                let v = match r.below(4) {
                    0 => money(cents(r, 1, 999_999, 1)),
                    1 => money_commas(cents(r, 100_000, 99_999_999, 1)),
                    2 => hm(time(r)),
                    _ => range(r, 1, 999).to_string(),
                };
                s.push_str(&v);
                s.push_str(if i + 1 == k {
                    "."
                } else {
                    pick(r, &[", ", "; ", ", ", ": "])
                });
            }
            s
        }
        22 => n_of(r, 5, 7, |r| {
            let v = match r.below(5) {
                0 => some_digits(r, 3, 3),
                1 => money(cents(r, 1, 999_999, 1)),
                2 => hm(time(r)),
                3 => ymd(date(r), '-'),
                _ => format!("{}%", range(r, 1, 100)),
            };
            match r.below(5) {
                0 => format!("\"{v}\""),
                1 => format!("'{v}'"),
                2 => format!("{v}?"),
                3 => format!("{v}!"),
                _ => format!("\"{v}\"?"),
            }
        }),
        23 => {
            if r.below(3) == 0 {
                let (a, b, c) = (range(r, 1, 99), range(r, 1, 50), range(r, 1, 40));
                if r.below(2) == 0 {
                    format!("({a} + ({b} - {c})) = {}", a + b - c)
                } else {
                    format!("(({a} + {b}) * {c}) = {}", (a + b) * c)
                }
            } else {
                n_of(r, 4, 6, |r| match r.below(4) {
                    0 => format!("({})", range(r, 1, 999)),
                    1 => format!("(({}))", some_digits(r, 2, 2)),
                    2 => format!("({}+({}))", range(r, 1, 99), range(r, 1, 99)),
                    _ => format!("({})", money(cents(r, 1, 99_999, 5))),
                })
            }
        }
        24 => n_of(r, 4, 6, |r| match r.below(6) {
            0 => format!("[{}]", range(r, 1, 999)),
            1 => format!("{{{}}}", range(r, 1, 999)),
            2 => format!("[{{{}}}]", any_digits(r, 2)),
            3 => format!("{{[{}]}}", any_digits(r, 3)),
            4 => format!(
                "[{}, {}, {}]",
                range(r, 1, 99),
                range(r, 1, 99),
                range(r, 1, 99)
            ),
            _ => format!("{{{}; {}}}", range(r, 1, 999), range(r, 1, 999)),
        }),
        25 => n_of(r, 4, 6, |r| {
            let sep = pick(r, &["/", "\\", "|", "_"]);
            let k = 2 + r.below(3);
            let parts: Vec<String> = (0..k).map(|_| some_digits(r, 2, 2)).collect();
            parts.join(sep)
        }),
        26 => n_of(r, 4, 6, |r| {
            let a = range(r, 1, 900);
            let b = a + range(r, 1, 99);
            match r.below(4) {
                0 => format!("{a}-{b}"),
                1 => format!("{a}--{b}"),
                2 => ymd(date(r), '-'),
                _ => format!(
                    "{:02}-{:02}-{:02}",
                    range(r, 1, 99),
                    range(r, 1, 99),
                    range(r, 1, 99)
                ),
            }
        }),
        27 => n_of(r, 5, 8, |r| match r.below(8) {
            0 => format!("@{}", any_digits(r, 3)),
            1 => format!("#{}", range(r, 1, 999)),
            2 => format!("${}", money(cents(r, 100, 99_999, 5))),
            3 => format!("{}%", range(r, 1, 100)),
            4 => format!("{}&{}", range(r, 1, 99), range(r, 1, 99)),
            5 => format!("{}*{}", range(r, 1, 99), range(r, 1, 99)),
            6 => format!("{}@{}", any_digits(r, 2), any_digits(r, 2)),
            _ => format!("#{}-{}", any_digits(r, 2), any_digits(r, 3)),
        }),
        28 => match r.below(6) {
            0 => sum_line(r, "Rs.", 2),
            1 => diff_line(r, "Rs."),
            2 => product_line(r, "Rs."),
            3 => division_line(r, "Rs."),
            4 => sum_line(r, "EUR ", 2),
            _ => n_of(r, 4, 6, |r| {
                let cur = pick(r, &["Rs.", "Rs.", "$", "EUR "]);
                format!("{cur}{}", money_commas(cents(r, 50, 9_999_999, 5)))
            }),
        },
        29 => match r.below(5) {
            0 => format!(
                "{}; {}: ({}).",
                money_commas(cents(r, 100_000, 9_999_999, 1)),
                money_commas(cents(r, 100_000, 9_999_999, 1)),
                money_commas(cents(r, 100_000, 9_999_999, 1))
            ),
            1 => format!(
                "{}, {}; {}%.",
                dmy(date(r), '/'),
                hm(time(r)),
                range(r, 1, 99)
            ),
            2 => format!(
                "[{:03}-{:03}] {{{:03}-{:03}}}.",
                range(r, 1, 300),
                range(r, 301, 600),
                range(r, 1, 300),
                range(r, 301, 600)
            ),
            3 => format!(
                "\"{}\"; [{}]; {{{}}}.",
                money(cents(r, 100, 99_999, 5)),
                money(cents(r, 100, 99_999, 5)),
                money(cents(r, 100, 99_999, 5))
            ),
            _ => format!("{}; ({}).", sum_line(r, "", 2), any_digits(r, 3)),
        },
        30 => {
            let k = [21, 22, 23, 24, 25, 26, 27, 29][r.below(8)];
            line_for(k, r)
        }
        31 => match r.below(3) {
            0 => {
                let terms = 2 + r.below(2);
                sum_line(r, "", terms)
            }
            1 => {
                let price = cents(r, 500, 190_000, 5);
                let note = [10_000, 20_000, 50_000, 100_000, 200_000]
                    .into_iter()
                    .find(|n| *n >= price)
                    .unwrap_or(200_000);
                format!(
                    "{} - {} = {}",
                    money(note),
                    money(price),
                    money(note - price)
                )
            }
            _ => product_line(r, ""),
        },
        32 => {
            let mut bal = cents(r, 100_000, 5_000_000, 50);
            let dep = cents(r, 10_000, 1_000_000, 25);
            let wd = cents(r, 5_000, bal + dep, 25);
            let a = format!("{} + {} = {}", money(bal), money(dep), money(bal + dep));
            bal += dep;
            format!("{a}; {} - {} = {}", money(bal), money(wd), money(bal - wd))
        }
        33 => tax_line(r),
        34 => interest_line(r),
        35 => {
            let open = cents(r, 100_000, 9_000_000, 50);
            let rec = cents(r, 10_000, 2_000_000, 25);
            let pay = cents(r, 5_000, open + rec, 25);
            format!(
                "{} + {} - {} = {}",
                money(open),
                money(rec),
                money(pay),
                money(open + rec - pay)
            )
        }
        36 => String::new(), // ledger blocks are built whole, see `ledger_block`
        37 => match r.below(4) {
            0 => format!("**** **** **** {}", any_digits(r, 4)),
            1 => format!("****-****-****-{}", any_digits(r, 4)),
            2 => {
                let d = date(r);
                format!(
                    "****{}; {}{:02}{:02}{}",
                    any_digits(r, 4),
                    d.0,
                    d.1,
                    d.2,
                    any_digits(r, 4)
                )
            }
            _ => format!(
                "****{} | {} | {} {}",
                any_digits(r, 4),
                money(cents(r, 100, 999_999, 1)),
                ymd(date(r), '-'),
                hm(time(r))
            ),
        },
        38 => {
            let k = [31, 32, 33, 34, 35, 37, 13, 14][r.below(8)];
            line_for(k, r)
        }
        39 => {
            let a = range(r, 1, 40);
            match r.below(4) {
                0 => format!("{a}.1 ({}) {a}.2 ({}) {a}.3 ({}) {a}.4 ({})", 1, 2, 3, 4),
                1 => {
                    let b = range(r, 1, 9);
                    format!(
                        "{a}.{b}.1 [{:03}] {a}.{b}.2 [{:03}] {a}.{b}.3 [{:03}]",
                        range(r, 1, 300),
                        range(r, 301, 600),
                        range(r, 601, 999)
                    )
                }
                2 => format!(
                    "({a}.1-{a}.{}) [{:03}-{:03}] {{{a}.{}}}",
                    range(r, 2, 9),
                    range(r, 1, 400),
                    range(r, 401, 999),
                    range(r, 1, 9)
                ),
                _ => format!(
                    "{a}.1: {a}.2; {a}.3, {a}.4. ({}) [{:03}]",
                    range(r, 1, 9),
                    range(r, 1, 999)
                ),
            }
        }
        41 => String::new(), // form rows are built whole, see `form_rows`
        42 => String::new(), // stock rows are built whole, see `stock_rows`
        43 => match r.below(4) {
            0 => {
                let step = [5, 10, 15, 20, 25, 30][r.below(6)];
                let mut t = range(r, 0, 20 * 60);
                let mut out = Vec::new();
                while out.len() < 6 && t < 24 * 60 {
                    out.push(hm((t / 60, t % 60)));
                    t += step;
                }
                join(out)
            }
            1 => format!(
                "{} {} {} {}",
                ymd(date(r), '-'),
                hms(r),
                ymd(date(r), '-'),
                hms(r)
            ),
            2 => format!(
                "{} {} {} {}",
                dmy(date(r), '/'),
                hm(time(r)),
                dmy(date(r), '/'),
                hm(time(r))
            ),
            _ => n_of(r, 3, 4, time_band),
        },
        44 => match r.below(4) {
            0 => format!(
                "1. {}; 2. {}; 3. {}.",
                dmy(date(r), '/'),
                dmy(date(r), '/'),
                dmy(date(r), '/')
            ),
            1 => format!("(1) {}; (2) {}.", time_band(r), time_band(r)),
            2 => format!(
                "[{:02}] Rs.{}; [{:02}] Rs.{}.",
                range(r, 1, 49),
                money(cents(r, 100, 999_999, 5)),
                range(r, 50, 99),
                money(cents(r, 100, 999_999, 5))
            ),
            _ => format!(
                "{}/{}; {}.",
                range(r, 2024, 2028),
                range(r, 100, 999),
                dmy(date(r), '/')
            ),
        },
        45 => match r.below(4) {
            0 => {
                let d = date(r);
                format!(
                    "{}|{}|{}|{}",
                    any_digits(r, 6),
                    money(cents(r, 100, 999_999, 1)),
                    dmy(d, '/'),
                    hm(time(r))
                )
            }
            1 => n_of(r, 4, 6, |r| {
                let d = date(r);
                format!("{:02}{:02}{}", d.2, d.1, d.0)
            }),
            2 => {
                let parts: Vec<String> = (0..5).map(|_| money(cents(r, 100, 999_999, 1))).collect();
                parts.join(",")
            }
            _ => format!(
                "Rs.{}|Rs.{}|Rs.{}",
                money(cents(r, 100, 999_999, 5)),
                money(cents(r, 100, 999_999, 5)),
                money(cents(r, 100, 999_999, 5))
            ),
        },
        46 => {
            let a = range(r, 1, 30);
            match r.below(4) {
                0 => format!(
                    "{a}.1 (1) [{:03}]; {a}.2 (2) {{{}}}.",
                    range(r, 1, 999),
                    money(cents(r, 100, 99_999, 5))
                ),
                1 => {
                    let x = cents(r, 1_000, 99_999, 5);
                    let y = cents(r, 1_000, 99_999, 5);
                    format!(
                        "{a}.1 [{}]; ({} + {}) = {}.",
                        dmy(date(r), '/'),
                        money(x),
                        money(y),
                        money(x + y)
                    )
                }
                2 => format!("{a}.1 -- {a}.4; {a}.5 - {a}.8 ({}).", time_band(r)),
                _ => format!(
                    "{a}.1 {{{}, {}, Rs.{}}}.",
                    ymd(date(r), '/'),
                    hm(time(r)),
                    money(cents(r, 100, 99_999, 5))
                ),
            }
        }
        49 => {
            let k = [41, 45, 31, 13, 14, 28, 37, 25][r.below(8)];
            if k == 41 {
                let serial = range(r, 1, 999);
                form_row(r, serial)
            } else {
                line_for(k, r)
            }
        }
        _ => {
            // Every family except the ratio drill (its nn:nn pairs are not
            // clock times, which the final assessment does check).
            let families = [
                1, 3, 5, 7, 11, 13, 14, 15, 16, 18, 19, 21, 22, 23, 24, 25, 27, 28, 31, 33, 34, 35,
                37, 39, 43, 45,
            ];
            let k = families[r.below(families.len())];
            line_for(k, r)
        }
    }
}

fn form_row(r: &mut Rng, serial: i64) -> String {
    format!(
        "{serial:03} | {} | {} | {}",
        dmy(date(r), '/'),
        money(cents(r, 100, 999_999, 5)),
        hm(time(r))
    )
}

/// A ledger block: running balance from zero, debit or credit per row,
/// then the debit and credit totals and the closing check.
fn ledger_block(r: &mut Rng, first_ref: i64) -> Vec<String> {
    let rows = 6 + r.below(5);
    let mut bal = 0;
    let mut deb_total = 0;
    let mut cred_total = 0;
    let mut out = vec![format!(
        "Ledger block {first_ref:03}: opening balance 0.00."
    )];
    for k in 0..rows {
        let reference = first_ref + i64::try_from(k).unwrap_or(0);
        let (d, c) = if k == 0 || r.below(2) == 0 || bal < 1_000 {
            (cents(r, 1_000, 500_000, 5), 0)
        } else {
            (0, cents(r, 500, bal, 5))
        };
        bal += d - c;
        deb_total += d;
        cred_total += c;
        out.push(format!(
            "{reference:03} | {} | {} | {}",
            money(d),
            money(c),
            money(bal)
        ));
    }
    out.push(format!(
        "Debits {}; credits {}; closing {}.",
        money(deb_total),
        money(cred_total),
        money(bal)
    ));
    out.push(format!(
        "{} - {} = {}",
        money(deb_total),
        money(cred_total),
        money(bal)
    ));
    out
}

fn stock_row(r: &mut Rng, item: i64) -> String {
    let opening = range(r, 20, 900);
    if r.below(2) == 0 {
        let issued = range(r, 0, opening);
        format!(
            "{item:03} | {opening:03} | {issued:03} | {:03}",
            opening - issued
        )
    } else {
        let received = range(r, 0, 400);
        let issued = range(r, 0, opening + received);
        format!(
            "{item:03} | {opening:03} | {received:03} | {issued:03} | {:03}",
            opening + received - issued
        )
    }
}

fn words(lines: &[String]) -> usize {
    lines.iter().map(|l| l.split_whitespace().count()).sum()
}

/// Build the 50 drills. `seed_sha` is the sha256 of the seed file.
///
/// # Errors
///
/// When a drill cannot reach [`MIN_WORDS`] with unique lines, or any line
/// fails the numeric check.
pub fn build(seeds: &[Seed], seed_sha: &str) -> anyhow::Result<(Vec<ImportedExercise>, Notes)> {
    let mut notes = Notes::default();
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (i, seed) in seeds.iter().enumerate() {
        let n = i + 1;
        let rules = numeric::rules_for(&seed.id);
        let mut lines: Vec<String> = Vec::new();
        if let Some(note) = key_note(&seed.id) {
            lines.push(note.to_owned());
            used.insert(note.to_owned());
        }
        for raw in &seed.lines {
            let mut line = ascii_line(raw, &mut notes.mapped);
            if let Some(fixed) = seed_fix(&seed.id, &line) {
                notes
                    .fixed
                    .push(format!("{}: {line:?} -> {fixed:?}", seed.id));
                line = fixed.to_owned();
            }
            if !used.insert(line.clone()) {
                notes.repeated_seed_lines += 1;
                continue;
            }
            lines.push(line);
        }
        let mut r = Rng::new(0xA1_0000 + u64::try_from(n).unwrap_or(0) * 0x9E37_79B9);
        let mut serial = 100 + i64::try_from(n).unwrap_or(0) * 10;
        let mut misses = 0;
        while words(&lines) < MIN_WORDS {
            let batch: Vec<String> = match n {
                36 => {
                    serial += 20;
                    ledger_block(&mut r, serial)
                }
                41 => {
                    serial += 1;
                    vec![form_row(&mut r, serial)]
                }
                42 => {
                    serial += 1;
                    vec![stock_row(&mut r, serial % 1000)]
                }
                _ => vec![line_for(n, &mut r)],
            };
            if batch.iter().any(|l| l.is_empty() || used.contains(l)) {
                misses += 1;
                if misses > 5_000 {
                    bail!("{}: cannot find enough unique lines", seed.id);
                }
                continue;
            }
            for l in batch {
                used.insert(l.clone());
                lines.push(l);
            }
        }
        let content = lines.join("\n");
        let problems = numeric::check_text(&content, rules);
        if !problems.is_empty() {
            bail!(
                "{} fails the numeric check:\n  {}",
                seed.id,
                problems.join("\n  ")
            );
        }
        let mut e = ImportedExercise {
            number: u32::try_from(n).context("count")?,
            title: seed.title.clone(),
            category: Some(seed.category.clone()),
            difficulty: Some(u8::try_from(1 + i / 10).unwrap_or(5)),
            content,
            source_id: seed.id.clone(),
            status: ContentStatus::GeneratedReviewPending,
            source_path: format!(
                "radha-track03:data/track03-alphanumeric-50-seed.md#{}",
                seed.id
            ),
            source_sha256: seed_sha.to_owned(),
            ..ImportedExercise::default()
        };
        e.seal();
        out.push(e);
    }
    Ok((out, notes))
}

/// Read, expand and check the seed file.
///
/// # Errors
///
/// As [`parse_seeds`] and [`build`].
pub fn from_seed_file(md: &str) -> anyhow::Result<(Vec<ImportedExercise>, Notes)> {
    let seeds = parse_seeds(md)?;
    if seeds.len() != 50 {
        bail!("{} seed drills (50 expected)", seeds.len());
    }
    build(&seeds, &sha256_hex(md.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_mapping_is_counted_and_spaces_collapse() {
        let mut m = Vec::new();
        assert_eq!(
            ascii_line("₹25.00 × 4 = ₹100.00", &mut m),
            "Rs.25.00 * 4 = Rs.100.00"
        );
        assert_eq!(ascii_line("10–20  100—200", &mut m), "10-20 100--200");
        assert_eq!(ascii_line("€1.00", &mut m), "EUR 1.00");
        assert!(m.contains(&('₹', 2)));
    }

    #[test]
    fn money_formats() {
        assert_eq!(money(125_050), "1250.50");
        assert_eq!(money_commas(123_456_789), "1,234,567.89");
        assert_eq!(money_commas(5), "0.05");
        assert_eq!(rate_of(73_150, 50), 3_658, "36.575 rounds half up");
    }
}

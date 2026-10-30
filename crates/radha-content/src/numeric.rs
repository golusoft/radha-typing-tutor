//! Checks for numeric drill text (Track 03 · Number + Punctuation): every
//! written calculation is right, every date is a real calendar date and every
//! clock time is valid.
//!
//! ## Calculations
//!
//! A line is split into statements at `;`. A statement holding `=` is a chain
//! of expressions (`a = b = c`) whose sides must all be equal. Expressions use
//! `+ - * /`, `:` (a ratio, read as division), `%`, brackets `() [] {}` and
//! numbers with optional thousands commas, decimals, sign and a currency
//! prefix (`Rs.`, `$`, `EUR`). Percent arithmetic is the office convention:
//! `a + p%` is `a` increased by `p` percent, `a - p%` decreased, `a * p%` is
//! `p` percent of `a`, and a plain value compared with a percent compares it
//! times 100 (`45/180 = 25%`).
//!
//! Rounding is stated once and applied everywhere: a written result with `d`
//! decimals must equal the exact value rounded half away from zero to `d`
//! decimals (`731.50 * 5% = 36.58`, exact 36.575). Two computed sides must be
//! exactly equal.
//!
//! ## Dates and times
//!
//! `dd/mm/yyyy`, `dd-mm-yyyy`, `dd.mm.yyyy` and `yyyy-mm-dd` (any of the three
//! separators) with a year 1900–2099 are dates and must exist on the
//! calendar. `hh:mm` and `hh:mm:ss` (two digits each) are times, 00:00–23:59.
//! In a ratio drill two-digit pairs such as `24:32` are ratios, not times.
//!
//! ## Tables
//!
//! Opt in per drill ([`Rules`]): a ledger row `ref | debit | credit |
//! balance` must carry the running balance of its block, and an inventory row
//! `item | opening | issued | closing` (or `item | opening | received |
//! issued | closing`) its closing stock.

/// Which optional checks a drill gets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rules {
    /// `nn:nn` pairs are ratios (no time check).
    pub ratios: bool,
    /// Four-column decimal rows are a running-balance ledger.
    pub ledger: bool,
    /// Four- or five-column integer rows are stock rows.
    pub inventory: bool,
}

/// The checks of a Track 03 drill, by its exercise id (`ALN-017` …).
#[must_use]
pub fn rules_for(id: &str) -> Rules {
    Rules {
        ratios: id == "ALN-017",
        ledger: id == "ALN-036",
        inventory: id == "ALN-042",
    }
}

/// Every problem found in `text`, as `line N: detail`. Empty when the text
/// is correct.
#[must_use]
pub fn check_text(text: &str, rules: Rules) -> Vec<String> {
    let mut problems = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        for detail in check_line(line, rules) {
            problems.push(format!("line {n}: {detail}: {line:?}"));
        }
    }
    if rules.ledger || rules.inventory {
        for detail in check_tables(text, rules) {
            problems.push(detail);
        }
    }
    problems
}

/// Problems on one line: calculations, dates and times.
#[must_use]
pub fn check_line(line: &str, rules: Rules) -> Vec<String> {
    let mut out = Vec::new();
    for statement in line.split(';') {
        if let Err(e) = check_statement(statement) {
            out.push(e);
        }
    }
    for date in dates_in(line) {
        if !date.valid() {
            out.push(format!("not a calendar date {}", date.text));
        }
    }
    if !rules.ratios {
        for time in times_in(line) {
            if !time.1 {
                out.push(format!("not a valid time {}", time.0));
            }
        }
    }
    out
}

// --- exact arithmetic ----------------------------------------------------------

/// An exact rational `n / d`, `d > 0`, reduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Q {
    n: i128,
    d: i128,
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

impl Q {
    fn new(n: i128, d: i128) -> Option<Q> {
        if d == 0 {
            return None;
        }
        let s = if d < 0 { -1 } else { 1 };
        let g = gcd(n, d);
        Some(Q {
            n: s * n / g,
            d: s * d / g,
        })
    }
    fn int(n: i128) -> Q {
        Q { n, d: 1 }
    }
    fn add(self, o: Q) -> Option<Q> {
        Q::new(
            self.n
                .checked_mul(o.d)?
                .checked_add(o.n.checked_mul(self.d)?)?,
            self.d.checked_mul(o.d)?,
        )
    }
    fn neg(self) -> Q {
        Q {
            n: -self.n,
            d: self.d,
        }
    }
    fn mul(self, o: Q) -> Option<Q> {
        Q::new(self.n.checked_mul(o.n)?, self.d.checked_mul(o.d)?)
    }
    fn div(self, o: Q) -> Option<Q> {
        Q::new(self.n.checked_mul(o.d)?, self.d.checked_mul(o.n)?)
    }
    /// Rounded half away from zero to `places` decimals.
    fn round(self, places: u32) -> Option<Q> {
        let scale = 10_i128.checked_pow(places)?;
        let scaled = self.n.checked_mul(scale)?;
        let q = scaled / self.d;
        let r = scaled % self.d;
        let up = r.abs().checked_mul(2)? >= self.d;
        let q = if up { q + scaled.signum() } else { q };
        Q::new(q, scale)
    }
}

/// A value: a number or a percentage (`p` of `p%`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Val {
    Num(Q),
    Pct(Q),
}

/// One side of an equation: its value and, when it is a single written
/// number, that number's decimal places (the rounding target).
struct Side {
    val: Val,
    literal_places: Option<u32>,
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    /// Tokens seen (a single number token makes a literal side).
    tokens: usize,
    places: u32,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser {
            s: s.as_bytes(),
            i: 0,
            tokens: 0,
            places: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i] == b' ' {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.s.get(self.i).copied()
    }

    fn expr(&mut self) -> Result<Val, String> {
        let mut acc = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            self.tokens += 1;
            let rhs = self.term()?;
            acc = combine_add(acc, rhs, op == b'-')?;
        }
        Ok(acc)
    }

    fn term(&mut self) -> Result<Val, String> {
        let mut acc = self.factor()?;
        while let Some(op @ (b'*' | b'/' | b':')) = self.peek() {
            self.i += 1;
            self.tokens += 1;
            let rhs = self.factor()?;
            acc = combine_mul(acc, rhs, op != b'*')?;
        }
        Ok(acc)
    }

    fn factor(&mut self) -> Result<Val, String> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok(match self.factor()? {
                    Val::Num(q) => Val::Num(q.neg()),
                    Val::Pct(q) => Val::Pct(q.neg()),
                })
            }
            Some(b'+') => {
                self.i += 1;
                self.factor()
            }
            Some(open @ (b'(' | b'[' | b'{')) => {
                self.i += 1;
                self.tokens += 2;
                let close = match open {
                    b'(' => b')',
                    b'[' => b']',
                    _ => b'}',
                };
                let v = self.expr()?;
                if self.peek() != Some(close) {
                    return Err("unbalanced bracket".into());
                }
                self.i += 1;
                self.percent(v)
            }
            _ => {
                self.currency();
                let v = self.number()?;
                self.percent(v)
            }
        }
    }

    fn percent(&mut self, v: Val) -> Result<Val, String> {
        if self.peek() == Some(b'%') {
            self.i += 1;
            return match v {
                Val::Num(q) => Ok(Val::Pct(q)),
                Val::Pct(_) => Err("double percent".into()),
            };
        }
        Ok(v)
    }

    fn currency(&mut self) {
        self.skip_ws();
        let rest = &self.s[self.i..];
        for prefix in [&b"Rs."[..], b"EUR", b"$"] {
            if rest.starts_with(prefix) {
                self.i += prefix.len();
                self.skip_ws();
                return;
            }
        }
    }

    fn number(&mut self) -> Result<Val, String> {
        self.skip_ws();
        let start = self.i;
        let mut digits: i128 = 0;
        let mut places: u32 = 0;
        let mut seen_dot = false;
        let mut any = false;
        while let Some(&c) = self.s.get(self.i) {
            match c {
                b'0'..=b'9' => {
                    digits = digits
                        .checked_mul(10)
                        .and_then(|d| d.checked_add(i128::from(c - b'0')))
                        .ok_or("number too long")?;
                    if seen_dot {
                        places += 1;
                    }
                    any = true;
                }
                // A thousands comma sits between digits.
                b',' if any
                    && !seen_dot
                    && self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit) => {}
                b'.' if any
                    && !seen_dot
                    && self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit) =>
                {
                    seen_dot = true;
                }
                _ => break,
            }
            self.i += 1;
        }
        if !any {
            return Err(format!(
                "expected a number at {:?}",
                String::from_utf8_lossy(&self.s[start..])
            ));
        }
        self.tokens += 1;
        self.places = places;
        let q = Q::new(digits, 10_i128.pow(places)).ok_or("bad number")?;
        Ok(Val::Num(q))
    }
}

fn combine_add(a: Val, b: Val, minus: bool) -> Result<Val, String> {
    let sign = |q: Q| if minus { q.neg() } else { q };
    let over = || "overflow".to_string();
    Ok(match (a, b) {
        (Val::Num(x), Val::Num(y)) => Val::Num(x.add(sign(y)).ok_or_else(over)?),
        (Val::Pct(x), Val::Pct(y)) => Val::Pct(x.add(sign(y)).ok_or_else(over)?),
        (Val::Num(x), Val::Pct(p)) => {
            let factor = Q::int(1)
                .add(sign(p).div(Q::int(100)).ok_or_else(over)?)
                .ok_or_else(over)?;
            Val::Num(x.mul(factor).ok_or_else(over)?)
        }
        (Val::Pct(_), Val::Num(_)) => return Err("percent plus a plain number".into()),
    })
}

fn combine_mul(a: Val, b: Val, divide: bool) -> Result<Val, String> {
    let over = || "overflow".to_string();
    let hundred = Q::int(100);
    Ok(match (a, b, divide) {
        (Val::Num(x), Val::Num(y), false) => Val::Num(x.mul(y).ok_or_else(over)?),
        (Val::Num(x), Val::Num(y), true) => Val::Num(x.div(y).ok_or("division by zero")?),
        (Val::Num(x), Val::Pct(p), false) | (Val::Pct(p), Val::Num(x), false) => {
            Val::Num(x.mul(p).and_then(|v| v.div(hundred)).ok_or_else(over)?)
        }
        (Val::Pct(p), Val::Num(x), true) => Val::Pct(p.div(x).ok_or("division by zero")?),
        _ => return Err("unsupported percent operation".into()),
    })
}

fn parse_side(text: &str) -> Result<Side, String> {
    let mut p = Parser::new(text);
    let val = p.expr()?;
    p.skip_ws();
    if p.i != p.s.len() {
        return Err(format!(
            "cannot read {:?}",
            String::from_utf8_lossy(&p.s[p.i..])
        ));
    }
    let literal_places = (p.tokens == 1).then_some(p.places);
    Ok(Side {
        val,
        literal_places,
    })
}

/// Whether two sides agree under the rounding rule.
fn sides_agree(a: &Side, b: &Side) -> bool {
    // Bring both to the same kind: a plain value against a percent is the
    // value times 100.
    let (x, y) = match (a.val, b.val) {
        (Val::Num(x), Val::Num(y)) | (Val::Pct(x), Val::Pct(y)) => (x, y),
        (Val::Num(x), Val::Pct(y)) => match x.mul(Q::int(100)) {
            Some(x) => (x, y),
            None => return false,
        },
        (Val::Pct(x), Val::Num(y)) => match y.mul(Q::int(100)) {
            Some(y) => (x, y),
            None => return false,
        },
    };
    match (a.literal_places, b.literal_places) {
        (Some(_), Some(_)) | (None, None) => x == y,
        (None, Some(places)) => x.round(places) == Some(y),
        (Some(places), None) => y.round(places) == Some(x),
    }
}

/// Check one `;`-separated statement. Statements without `=` pass.
fn check_statement(statement: &str) -> Result<(), String> {
    if !statement.contains('=') {
        return Ok(());
    }
    let mut s = statement.trim();
    // A sentence-ending full stop after the last number.
    if let Some(stripped) = s.strip_suffix('.') {
        s = stripped;
    }
    let sides: Vec<Side> = s
        .split('=')
        .map(|side| parse_side(side.trim()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{e} in {statement:?}"))?;
    if sides.len() < 2 {
        return Err(format!("incomplete equation {statement:?}"));
    }
    for pair in sides.windows(2) {
        if !sides_agree(&pair[0], &pair[1]) {
            return Err(format!("wrong result in {:?}", statement.trim()));
        }
    }
    Ok(())
}

// --- dates and times -------------------------------------------------------------

/// A date found in text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundDate {
    /// The text as written.
    pub text: String,
    /// Year.
    pub year: u32,
    /// Month 1–12 (unchecked).
    pub month: u32,
    /// Day (unchecked).
    pub day: u32,
}

impl FoundDate {
    /// Whether the date exists.
    #[must_use]
    pub fn valid(&self) -> bool {
        (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
    }
}

/// Days in a month (`month` 1–12).
#[must_use]
pub fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

fn digit_run(b: &[u8], i: usize) -> usize {
    b[i..].iter().take_while(|c| c.is_ascii_digit()).count()
}

fn num(b: &[u8], from: usize, len: usize) -> u32 {
    b[from..from + len]
        .iter()
        .fold(0, |acc, c| acc * 10 + u32::from(c - b'0'))
}

/// Dates written in `text` (see the module docs for the formats).
#[must_use]
pub fn dates_in(text: &str) -> Vec<FoundDate> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() || (i > 0 && b[i - 1].is_ascii_digit()) {
            i += 1;
            continue;
        }
        let first = digit_run(b, i);
        let sep_ok = |at: usize| b.get(at).is_some_and(|c| matches!(c, b'/' | b'-' | b'.'));
        let mut matched = None;
        // yyyy s mm s dd
        if first == 4 && sep_ok(i + 4) {
            let j = i + 5;
            if j < b.len() && digit_run(b, j) == 2 && b.get(j + 2) == b.get(i + 4) {
                let k = j + 3;
                if k < b.len() && digit_run(b, k) == 2 {
                    matched = Some((num(b, i, 4), num(b, j, 2), num(b, k, 2), k + 2));
                }
            }
        }
        // dd s mm s yyyy
        if matched.is_none() && first == 2 && sep_ok(i + 2) {
            let j = i + 3;
            if j < b.len() && digit_run(b, j) == 2 && b.get(j + 2) == b.get(i + 2) {
                let k = j + 3;
                if k < b.len() && digit_run(b, k) == 4 {
                    matched = Some((num(b, k, 4), num(b, j, 2), num(b, i, 2), k + 4));
                }
            }
        }
        match matched {
            Some((year, month, day, end)) if (1900..=2099).contains(&year) => {
                out.push(FoundDate {
                    text: text[i..end].to_owned(),
                    year,
                    month,
                    day,
                });
                i = end;
            }
            _ => i += first.max(1),
        }
    }
    out
}

/// Clock times written in `text`, with whether each is valid.
#[must_use]
pub fn times_in(text: &str) -> Vec<(String, bool)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let starts = b[i].is_ascii_digit()
            && (i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b':' || b[i - 1] == b'.'));
        if !starts || digit_run(b, i) != 2 || b.get(i + 2) != Some(&b':') {
            i += 1;
            continue;
        }
        let j = i + 3;
        if j >= b.len() || digit_run(b, j) != 2 {
            i += 1;
            continue;
        }
        let mut end = j + 2;
        let mut parts = vec![num(b, i, 2), num(b, j, 2)];
        if b.get(end) == Some(&b':') && end + 1 < b.len() && digit_run(b, end + 1) == 2 {
            parts.push(num(b, end + 1, 2));
            end += 3;
        }
        // "12:30.5" or "12:345" are not times.
        if b.get(end).is_some_and(|c| c.is_ascii_digit() || *c == b':') {
            i = end;
            continue;
        }
        let valid = parts[0] <= 23 && parts[1..].iter().all(|&p| p <= 59);
        out.push((text[i..end].to_owned(), valid));
        i = end;
    }
    out
}

// --- tables ------------------------------------------------------------------------

fn decimal_cents(field: &str) -> Option<i128> {
    let (whole, frac) = field.split_once('.')?;
    if frac.len() != 2 || whole.is_empty() {
        return None;
    }
    let all = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
    if !all(whole) || !all(frac) {
        return None;
    }
    Some(whole.parse::<i128>().ok()? * 100 + frac.parse::<i128>().ok()?)
}

fn integer(field: &str) -> Option<i128> {
    if field.is_empty() || !field.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    field.parse().ok()
}

fn check_tables(text: &str, rules: Rules) -> Vec<String> {
    let mut out = Vec::new();
    let mut balance: Option<i128> = None;
    for (i, line) in text.lines().enumerate() {
        let fields: Vec<&str> = line.split('|').map(str::trim).collect();
        if rules.ledger {
            let row = (fields.len() == 4)
                .then(|| {
                    Some((
                        integer(fields[0])?,
                        decimal_cents(fields[1])?,
                        decimal_cents(fields[2])?,
                        decimal_cents(fields[3])?,
                    ))
                })
                .flatten();
            match row {
                Some((_, debit, credit, bal)) => {
                    let expected = balance.unwrap_or(0) + debit - credit;
                    if expected != bal {
                        out.push(format!(
                            "line {}: ledger balance should be {}.{:02}: {line:?}",
                            i + 1,
                            expected / 100,
                            expected % 100
                        ));
                    }
                    balance = Some(bal);
                }
                None => balance = None,
            }
        }
        if rules.inventory {
            let ints: Option<Vec<i128>> = fields.iter().map(|f| integer(f)).collect();
            if let Some(v) = ints {
                let ok = match v.len() {
                    4 => v[1] - v[2] == v[3],
                    5 => v[1] + v[2] - v[3] == v[4],
                    _ => true,
                };
                if !ok {
                    out.push(format!("line {}: stock does not balance: {line:?}", i + 1));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(line: &str) -> bool {
        check_line(line, Rules::default()).is_empty()
    }

    #[test]
    fn sums_and_differences() {
        assert!(ok("125.50 + 240.00 = 365.50"));
        assert!(ok("12500.00 + 2500.00 - 1750.00 = 13250.00"));
        assert!(ok("1,250.00 + 2,450.75 = 3,700.75."));
        assert!(ok("Rs.12.50 + Rs.7.50 = Rs.20.00"));
        assert!(ok("EUR 10.00 + EUR 5.50 = EUR 15.50"));
        assert!(ok("125.50+240.25=365.75; 500.00-125.00=375.00"));
        assert!(!ok("125.50 + 240.00 = 365.00"));
        assert!(!ok("1250.00 + 2200.00 = 3450.01"));
    }

    #[test]
    fn products_percentages_and_rounding() {
        assert!(ok("6 * 25.00 = 150.00"));
        assert!(ok("Rs.480.00 / 4 = Rs.120.00"));
        assert!(ok("1250.00 * 18% = 225.00; 1250.00 + 225.00 = 1475.00"));
        assert!(ok("731.50 * 5% = 36.58"), "36.575 rounds half up");
        assert!(!ok("731.50 * 5% = 36.57"));
        assert!(ok("1200 + 12.5% = 1350"));
        assert!(ok("2000 - 7.5% = 1850"));
        assert!(ok("45/180 = 25%"));
        assert!(ok("1.25% + 2.50% = 3.75%"));
        assert!(!ok("18/120 = 16%"));
        assert!(ok("(125.50 + 240.25) = 365.75"));
        assert!(ok("[1000.00 - 275.50] = 724.50"));
        assert!(ok("((10 - 2) * 3) = 24"));
    }

    #[test]
    fn fraction_and_ratio_chains_are_exact() {
        assert!(ok("3/5 = 6/10 = 9/15"));
        assert!(ok("2:3 = 4:6 = 6:9"));
        assert!(ok("1/8 = 0.125"));
        // A written result is the exact value rounded to its decimals.
        assert!(ok("1/3 = 0.33"));
        assert!(!ok("1/3 = 0.34"));
        assert!(!ok("1/3 = 2/6 = 0.4"));
    }

    #[test]
    fn unreadable_equations_fail() {
        assert!(!ok("Total: 5 = 5"));
        assert!(!ok("5 + = 5"));
    }

    #[test]
    fn dates_must_exist() {
        assert!(ok("28/02/2026 29/02/2028 31/12/2026 2026-11-07 07.11.2026"));
        assert!(!ok("29/02/2026"));
        assert!(!ok("31/04/2026"));
        assert!(!ok("07/17/2026"));
        assert!(!ok("2026-13-01"));
        // Reference numbers are not dates.
        assert!(ok("11/26/0042 21.06.0041 2026-00481 618/2026/014 12-34-56"));
        assert_eq!(dates_in("26/11/2026/001").len(), 1);
    }

    #[test]
    fn times_must_be_on_the_clock() {
        assert!(ok("00:00 09:15 23:59 08:00:05"));
        assert!(!ok("24:00"));
        assert!(!ok("12:60"));
        assert!(!ok("67:89?"));
        assert!(ok("123:45 1:2 5:8"), "not two-digit pairs");
        let ratios = Rules {
            ratios: true,
            ..Rules::default()
        };
        assert!(check_line("24:32 27:36", ratios).is_empty());
    }

    #[test]
    fn ledger_and_stock_rows_balance() {
        let ledger = Rules {
            ledger: true,
            ..Rules::default()
        };
        let good = "001 | 1250.00 | 0.00 | 1250.00\n002 | 0.00 | 450.00 | 800.00";
        assert!(check_text(good, ledger).is_empty());
        let bad = "001 | 1250.00 | 0.00 | 1250.00\n002 | 0.00 | 450.00 | 900.00";
        assert_eq!(check_text(bad, ledger).len(), 1);
        let stock = Rules {
            inventory: true,
            ..Rules::default()
        };
        assert!(check_text("001 | 025 | 012 | 013\n002 | 10 | 5 | 3 | 12", stock).is_empty());
        assert_eq!(check_text("001 | 025 | 012 | 014", stock).len(), 1);
    }
}

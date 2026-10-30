//! The utility overlays (Analytics, Profile, Settings, Candidate info) and
//! the Home hero figures (P08), wired to `UtilityView`.
//!
//! Every aggregate is read from SQLite when an overlay opens, when Home is
//! first shown and after a session is saved — never per key (`RULES.MD`
//! Rule 33). The pure formatting rules are plain functions with tests;
//! the window writes are thin.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use radha_analytics::{
    achievements, by_layout, in_window, level, streak, window_start, AchievementInput,
    AnalyticsProvider, AnalyticsSummary, DayPoint, LayoutAggregate, ProductAggregate,
};
use slint::{ComponentHandle, ModelRc, VecModel};

use super::flow::Flow;
use super::{
    AchievementLine, AppWindow, HistoryLine, Motion, PerfRow, StatLine, Theme, UtilityView,
};
use crate::report::chart_path;
use crate::settings::Settings;

/// Per-profile value names (`settings` table).
pub const PHOTO_KEY: &str = "profile.photo";
pub const CANDIDATE_NAME_KEY: &str = "candidate.name";
pub const CANDIDATE_ROLL_KEY: &str = "candidate.roll";

/// Sessions listed in the history.
const HISTORY_ROWS: u32 = 12;
/// Days read for streaks.
const STREAK_DAYS: u32 = 400;

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// Today's day, on the same (UTC) calendar `save_session` records.
#[must_use]
pub fn today() -> String {
    crate::practice::iso_utc(std::time::SystemTime::now())
        .chars()
        .take(10)
        .collect()
}

/// "1h 05m", "12m", "45s", "—" for none.
#[must_use]
pub fn duration_text(seconds: u64) -> String {
    match seconds {
        0 => "—".to_owned(),
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}

fn opt(v: Option<f64>, unit: &str) -> String {
    v.filter(|x| x.is_finite())
        .map_or_else(|| "—".to_owned(), |x| format!("{x:.0}{unit}"))
}

/// Days of a range id.
#[must_use]
pub fn range_days(id: &str) -> u32 {
    if id == "30d" {
        30
    } else {
        7
    }
}

/// A day-series chart: one point per practice day in the window (no
/// invented zero days), the latest value, and the change across it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Series {
    pub chart: Option<crate::report::ChartPath>,
    pub value: String,
    pub footer: String,
    pub trend: String,
}

/// Build a [`Series`] from `points` with `pick` (net WPM or accuracy).
#[must_use]
pub fn series(points: &[DayPoint], days: u32, unit: &str, pick: fn(&DayPoint) -> f64) -> Series {
    let values: Vec<f64> = points.iter().map(pick).collect();
    let Some(&last) = values.last() else {
        return Series {
            footer: format!("No sessions in the last {days} days"),
            ..Series::default()
        };
    };
    let sessions: u32 = points.iter().map(|p| p.sessions).sum();
    let trend = match values.first() {
        Some(&first) if values.len() > 1 => {
            let d = last - first;
            if d.abs() < 0.5 {
                "steady".to_owned()
            } else {
                format!(
                    "{}{:.0}{unit} vs first day",
                    if d > 0.0 { "+" } else { "−" },
                    d.abs()
                )
            }
        }
        _ => String::new(),
    };
    Series {
        chart: chart_path(&values),
        value: format!("{last:.0}{unit} latest"),
        footer: format!(
            "{} practice day{} · {sessions} session{} · last {days} days",
            values.len(),
            if values.len() == 1 { "" } else { "s" },
            if sessions == 1 { "" } else { "s" }
        ),
        trend,
    }
}

fn perf_row(
    name: &str,
    detail: &str,
    sessions: u32,
    p: (Option<f64>, Option<f64>, Option<f64>),
    secs: u64,
) -> PerfRow {
    let (net, acc, best) = p;
    PerfRow {
        name: name.into(),
        detail: detail.into(),
        sessions: sessions.to_string().into(),
        net: opt(net, "").into(),
        accuracy: opt(acc, "%").into(),
        best: opt(best, "").into(),
        time: duration_text(secs).into(),
        has_data: sessions > 0,
    }
}

fn product_rows(rows: &[ProductAggregate]) -> Vec<PerfRow> {
    rows.iter()
        .map(|p| {
            perf_row(
                &p.product_name,
                &p.layout_name,
                p.sessions,
                (p.average_net_wpm, p.average_accuracy, p.best_net_wpm),
                p.practice_seconds,
            )
        })
        .collect()
}

fn layout_rows(rows: &[LayoutAggregate]) -> Vec<PerfRow> {
    rows.iter()
        .map(|l| {
            perf_row(
                &l.layout_name,
                "",
                l.sessions,
                (l.average_net_wpm, l.average_accuracy, l.best_net_wpm),
                l.practice_seconds,
            )
        })
        .collect()
}

fn stat(label: &str, value: String, note: String) -> StatLine {
    StatLine {
        label: label.into(),
        value: value.into(),
        note: note.into(),
    }
}

/// The eight Analytics KPI tiles.
#[must_use]
pub fn kpis(s: &AnalyticsSummary, streak_days: u32) -> Vec<(String, String, String)> {
    let f = |v: f64, unit: &str| format!("{v:.0}{unit}");
    vec![
        (
            "SESSIONS".into(),
            s.sessions.to_string(),
            format!("{streak_days}-day streak"),
        ),
        (
            "PRACTICE TIME".into(),
            duration_text(s.total_practice_seconds),
            "active typing".into(),
        ),
        (
            "AVG NET WPM".into(),
            f(s.average_net_wpm, ""),
            format!("gross {:.0}", s.average_wpm),
        ),
        (
            "BEST NET WPM".into(),
            f(s.best_net_wpm, ""),
            format!("gross {:.0}", s.best_wpm),
        ),
        (
            "AVG ACCURACY".into(),
            f(s.average_accuracy, "%"),
            "per session".into(),
        ),
        (
            "BEST ACCURACY".into(),
            f(s.best_accuracy, "%"),
            "one session".into(),
        ),
        (
            "CONSISTENCY".into(),
            opt(s.average_consistency, "%"),
            "steadiness of key rhythm".into(),
        ),
        (
            "EXERCISES".into(),
            s.exercises_completed.to_string(),
            format!("finished · {} practised", s.exercises_practised),
        ),
    ]
}

/// Shared state of the overlays.
pub struct Utility {
    window: slint::Weak<AppWindow>,
    db: Rc<RefCell<radha_db::Database>>,
    profile_id: i64,
    settings: Rc<RefCell<Settings>>,
    settings_path: PathBuf,
    data_dir: PathBuf,
    flow: Rc<RefCell<Flow>>,
}

impl Utility {
    /// Wire `UtilityView` and fill the Home hero.
    pub fn install(
        window: &AppWindow,
        db: Rc<RefCell<radha_db::Database>>,
        profile_id: i64,
        settings: Rc<RefCell<Settings>>,
        settings_path: PathBuf,
        data_dir: PathBuf,
        flow: Rc<RefCell<Flow>>,
    ) -> Rc<RefCell<Utility>> {
        let u = Rc::new(RefCell::new(Utility {
            window: window.as_weak(),
            db,
            profile_id,
            settings,
            settings_path,
            data_dir,
            flow,
        }));
        refresh_home(window, &u.borrow().db, profile_id);
        wire(window, &u);
        u
    }

    fn with_window(&self, f: impl FnOnce(&AppWindow)) {
        if let Some(w) = self.window.upgrade() {
            f(&w);
        }
    }

    /// Fill and show an overlay ("analytics", "profile", "settings",
    /// "candidate").
    pub fn open(&mut self, id: &str) {
        match id {
            "analytics" => self.fill_analytics(),
            "profile" => self.fill_profile(),
            "settings" => self.fill_settings(),
            "candidate" => self.fill_candidate(),
            _ => {
                log::warn!("Ignoring unknown overlay '{id}'");
                return;
            }
        }
        let id = id.to_owned();
        self.with_window(|w| w.global::<UtilityView>().set_open(id.into()));
    }

    fn close(&self) {
        self.with_window(|w| {
            w.global::<UtilityView>().set_open("".into());
            // Give the shell's key scope focus back (Escape, Ctrl+K).
            w.invoke_focus_keys();
        });
    }

    // --- Analytics -----------------------------------------------------------

    fn fill_analytics(&self) {
        let Ok(db) = self.db.try_borrow() else {
            return;
        };
        let p = self.profile_id;
        let warn = |what: &str, e: radha_core::CoreError| log::warn!("Analytics: {what}: {e}");
        let summary = db.summary(p).unwrap_or_else(|e| {
            warn("summary", e);
            AnalyticsSummary::default()
        });
        let range = self.window.upgrade().map_or_else(
            || "30d".to_owned(),
            |w| w.global::<UtilityView>().get_an_range().to_string(),
        );
        let days = range_days(&range);
        let today = today();
        let since = window_start(&today, days).unwrap_or_default();
        let points = in_window(
            &db.daily_points(p, &since).unwrap_or_else(|e| {
                warn("daily series", e);
                Vec::new()
            }),
            &today,
            days,
        );
        let speed = series(&points, days, " WPM", |d| d.average_net_wpm);
        let acc = series(&points, days, "%", |d| d.average_accuracy);
        let products = db.product_aggregates(p).unwrap_or_else(|e| {
            warn("products", e);
            Vec::new()
        });
        let layouts = by_layout(&products);
        let streak_days = streak(
            &db.practice_days(p, STREAK_DAYS).unwrap_or_default(),
            &today,
        );
        let weak: Vec<StatLine> = db
            .weak_key_totals(p, 8)
            .unwrap_or_default()
            .into_iter()
            .map(|k| {
                stat(
                    &k.physical_key,
                    format!("{}×", k.occurrences),
                    if k.expected_char.is_empty() {
                        "typed extra".to_owned()
                    } else {
                        format!("expected {}", k.expected_char)
                    },
                )
            })
            .collect();
        let e = db.error_totals(p).unwrap_or_default();
        let errors = if e.wrong + e.extra + e.missing == 0 && e.backspaces == 0 {
            Vec::new()
        } else {
            vec![
                stat(
                    "Wrong letter",
                    e.wrong.to_string(),
                    "a different key".into(),
                ),
                stat("Extra", e.extra.to_string(), "typed with no target".into()),
                stat("Missing", e.missing.to_string(), "skipped letters".into()),
                stat(
                    "Space errors",
                    e.space.to_string(),
                    "part of the three above".into(),
                ),
                stat(
                    "Backspaces",
                    e.backspaces.to_string(),
                    "corrections made".into(),
                ),
            ]
        };
        let history: Vec<HistoryLine> = db
            .recent_sessions(p, HISTORY_ROWS)
            .unwrap_or_default()
            .into_iter()
            .map(|s| HistoryLine {
                when: s
                    .completed_at
                    .replace('T', " ")
                    .chars()
                    .take(16)
                    .collect::<String>()
                    .into(),
                title: format!("{} · {}", s.exercise_number, s.exercise_title).into(),
                product: s.product_name.into(),
                net: format!("{:.0}", s.net_wpm).into(),
                accuracy: format!("{:.0}%", s.accuracy).into(),
                time: duration_text(u64::try_from(s.elapsed_secs).unwrap_or(0)).into(),
            })
            .collect();
        let tiles: Vec<StatLine> = kpis(&summary, streak_days)
            .into_iter()
            .map(|(l, v, n)| stat(&l, v, n))
            .collect();
        self.with_window(|w| {
            let v = w.global::<UtilityView>();
            v.set_an_has_data(summary.has_data());
            v.set_an_range(range.into());
            v.set_an_kpis(model(tiles));
            write_series(&speed, |c| {
                v.set_an_speed_has_data(c.is_some());
                if let Some(c) = c {
                    v.set_an_speed_line(c.line.as_str().into());
                    v.set_an_speed_area(c.area.as_str().into());
                    v.set_an_speed_peak_x(c.peak_x);
                    v.set_an_speed_peak_y(c.peak_y);
                }
            });
            v.set_an_speed_value(speed.value.as_str().into());
            v.set_an_speed_footer(speed.footer.as_str().into());
            write_series(&acc, |c| {
                v.set_an_acc_has_data(c.is_some());
                if let Some(c) = c {
                    v.set_an_acc_line(c.line.as_str().into());
                    v.set_an_acc_area(c.area.as_str().into());
                    v.set_an_acc_peak_x(c.peak_x);
                    v.set_an_acc_peak_y(c.peak_y);
                }
            });
            v.set_an_acc_value(acc.value.as_str().into());
            v.set_an_acc_footer(acc.footer.as_str().into());
            v.set_an_products(model(product_rows(&products)));
            v.set_an_layouts(model(layout_rows(&layouts)));
            v.set_an_weak(model(weak));
            v.set_an_errors(model(errors));
            v.set_an_history(model(history));
        });
    }

    // --- Profile ---------------------------------------------------------------

    fn fill_profile(&self) {
        let Ok(db) = self.db.try_borrow() else {
            return;
        };
        let p = self.profile_id;
        let name = db.profile_name(p).ok().flatten().unwrap_or_default();
        let photo = db
            .profile_value(p, PHOTO_KEY)
            .ok()
            .flatten()
            .unwrap_or_default();
        let summary = db.summary(p).unwrap_or_default();
        let today = today();
        let streak_days = streak(
            &db.practice_days(p, STREAK_DAYS).unwrap_or_default(),
            &today,
        );
        let smooth = db.best_flow_smoothness(p).ok().flatten();
        let has = summary.has_data();
        let lvl = level(summary.best_net_wpm, summary.sessions);
        let bests = vec![
            stat(
                "Best net WPM",
                if has {
                    format!("{:.0}", summary.best_net_wpm)
                } else {
                    "—".into()
                },
                String::new(),
            ),
            stat(
                "Best gross WPM",
                if has {
                    format!("{:.0}", summary.best_wpm)
                } else {
                    "—".into()
                },
                String::new(),
            ),
            stat(
                "Best accuracy",
                if has {
                    format!("{:.1}%", summary.best_accuracy)
                } else {
                    "—".into()
                },
                String::new(),
            ),
            stat(
                "Total practice",
                duration_text(summary.total_practice_seconds),
                String::new(),
            ),
            stat("Sessions", summary.sessions.to_string(), String::new()),
            stat(
                "Exercises finished",
                summary.exercises_completed.to_string(),
                String::new(),
            ),
            stat(
                "Practice streak",
                format!("{streak_days} days"),
                String::new(),
            ),
            stat("Best Flow smoothness", opt(smooth, "%"), String::new()),
        ];
        let ach: Vec<AchievementLine> = achievements(&AchievementInput {
            summary,
            streak: streak_days,
            best_smoothness: smooth,
        })
        .into_iter()
        .map(|a| AchievementLine {
            title: a.title.into(),
            progress: a.progress.into(),
            earned: a.earned,
        })
        .collect();
        let image = load_photo(&photo);
        self.with_window(|w| {
            let v = w.global::<UtilityView>();
            v.set_pf_initial(name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into());
            v.set_pf_name(name.as_str().into());
            v.set_pf_photo_path(photo.as_str().into());
            v.set_pf_has_photo(image.is_some());
            if let Some(i) = image {
                v.set_pf_photo(i);
            }
            v.set_pf_level(lvl.unwrap_or("—").into());
            v.set_pf_level_note(
                if lvl.is_some() {
                    "A practice band from your best net WPM: 20+ Developing, 30+ Intermediate, 40+ Advanced, 60+ Expert. Not an exam grade."
                } else {
                    "Your level appears after your first saved session."
                }
                .into(),
            );
            v.set_pf_bests(model(bests));
            v.set_pf_achievements(model(ach));
            v.set_pf_licence("Licence not configured".into());
            v.set_pf_status("".into());
        });
    }

    fn save_profile(&self, name: &str, photo: &str) {
        let photo = photo.trim().trim_matches('"');
        if !photo.is_empty() {
            if let Err(e) = check_photo(photo) {
                self.with_window(|w| w.global::<UtilityView>().set_pf_status(e.into()));
                return;
            }
        }
        let result = match self.db.try_borrow_mut() {
            Ok(mut db) => db
                .set_profile_name(self.profile_id, name)
                .and_then(|_| db.set_profile_values(self.profile_id, &[(PHOTO_KEY, photo)])),
            Err(_) => return,
        };
        match result {
            Ok(()) => {
                self.fill_profile();
                self.with_window(|w| w.global::<UtilityView>().set_pf_status("Saved.".into()));
            }
            Err(e) => {
                log::warn!("Could not save the profile: {e}");
                let msg = format!("Not saved: {e}");
                self.with_window(|w| w.global::<UtilityView>().set_pf_status(msg.into()));
            }
        }
    }

    // --- Settings ----------------------------------------------------------------

    fn fill_settings(&self) {
        let s = self.settings.borrow().clone();
        let config = self.settings_path.display().to_string();
        let data = self.data_dir.display().to_string();
        let dir = crate::export::default_dir(&self.data_dir);
        let stamp: String = crate::practice::iso_utc(std::time::SystemTime::now())
            .chars()
            .filter(char::is_ascii_digit)
            .take(12)
            .collect();
        let backup = dir
            .join(format!("radha-backup-{stamp}.sqlite"))
            .display()
            .to_string();
        let export = dir
            .join(format!("radha-history-{stamp}.csv"))
            .display()
            .to_string();
        self.with_window(|w| {
            write_settings(w, &s);
            let v = w.global::<UtilityView>();
            v.set_st_version(env!("CARGO_PKG_VERSION").into());
            v.set_st_config_path(config.into());
            v.set_st_data_path(data.into());
            v.set_st_backup_path(backup.into());
            v.set_st_export_path(export.into());
            v.set_st_data_status("".into());
        });
    }

    fn set_setting(&self, key: &str, value: &str) {
        let changed = self.settings.borrow_mut().set(key, value);
        if !changed {
            log::warn!("Ignoring invalid setting {key}={value}");
        }
        let s = self.settings.borrow().clone();
        if changed {
            if let Err(e) = s.save(&self.settings_path) {
                log::warn!("Could not save settings: {e}");
            }
            if let Ok(mut f) = self.flow.try_borrow_mut() {
                f.apply_settings(&s);
            }
        }
        self.with_window(|w| {
            apply_window_settings(w, &s);
            write_settings(w, &s);
        });
    }

    fn backup(&self, path: &str) {
        let status = match crate::export::validate_target(path, &["sqlite", "db"]) {
            Err(e) => e,
            Ok(target) => {
                let made = target
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .map_err(|e| e.to_string());
                match made.and_then(|()| match self.db.try_borrow() {
                    Ok(db) => db.backup_to(&target).map_err(|e| e.to_string()),
                    Err(_) => Err("the database is busy; try again".to_owned()),
                }) {
                    Ok(()) => {
                        log::info!("Database backed up to {}", target.display());
                        format!("Backup saved to {}", target.display())
                    }
                    Err(e) => {
                        log::warn!("Backup failed: {e}");
                        format!("Backup failed: {e}")
                    }
                }
            }
        };
        self.with_window(|w| w.global::<UtilityView>().set_st_data_status(status.into()));
    }

    fn export_history(&self, path: &str) {
        let status = match crate::export::validate_target(path, &["csv"]) {
            Err(e) => e,
            Ok(target) => {
                let rows = match self.db.try_borrow() {
                    Ok(db) => db
                        .recent_sessions(self.profile_id, u32::MAX)
                        .map_err(|e| e.to_string()),
                    Err(_) => Err("the database is busy; try again".to_owned()),
                };
                match rows.and_then(|r| {
                    let n = r.len();
                    crate::export::write_new(&target, crate::export::history_csv(&r).as_bytes())
                        .map(|()| n)
                        .map_err(|e| e.to_string())
                }) {
                    Ok(n) => format!("{n} sessions exported to {}", target.display()),
                    Err(e) => {
                        log::warn!("History export failed: {e}");
                        format!("Export failed: {e}")
                    }
                }
            }
        };
        self.with_window(|w| w.global::<UtilityView>().set_st_data_status(status.into()));
    }

    // --- Candidate info -------------------------------------------------------------

    fn fill_candidate(&self) {
        let Ok(db) = self.db.try_borrow() else {
            return;
        };
        let name = db
            .profile_value(self.profile_id, CANDIDATE_NAME_KEY)
            .ok()
            .flatten()
            .unwrap_or_default();
        let roll = db
            .profile_value(self.profile_id, CANDIDATE_ROLL_KEY)
            .ok()
            .flatten()
            .unwrap_or_default();
        self.with_window(|w| {
            let v = w.global::<UtilityView>();
            v.set_cd_name(name.into());
            v.set_cd_roll(roll.into());
            v.set_cd_status("".into());
        });
    }

    fn save_candidate(&self, name: &str, roll: &str) {
        let result = match self.db.try_borrow_mut() {
            Ok(mut db) => db.set_profile_values(
                self.profile_id,
                &[(CANDIDATE_NAME_KEY, name), (CANDIDATE_ROLL_KEY, roll)],
            ),
            Err(_) => return,
        };
        let status = match result {
            Ok(()) => "Saved. It appears on the scorecard you export.".to_owned(),
            Err(e) => format!("Not saved: {e}"),
        };
        if let Ok(f) = self.flow.try_borrow() {
            f.refresh_candidate();
        }
        self.fill_candidate();
        self.with_window(|w| w.global::<UtilityView>().set_cd_status(status.into()));
    }

    fn resume(&self) {
        let last = match self.db.try_borrow() {
            Ok(db) => db
                .recent_sessions(self.profile_id, 1)
                .ok()
                .and_then(|v| v.into_iter().next()),
            Err(_) => None,
        };
        let Some(last) = last else {
            return;
        };
        let Ok(product) = radha_content::ProductId::from_slug(&last.product_slug) else {
            return;
        };
        if let Ok(mut f) = self.flow.try_borrow_mut() {
            f.resume(product, last.exercise_id, last.mode.as_deref());
        }
    }
}

fn write_series(s: &Series, f: impl FnOnce(Option<&crate::report::ChartPath>)) {
    f(s.chart.as_ref());
}

/// Reject a photo path that is not an existing PNG or JPEG file.
fn check_photo(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !["png", "jpg", "jpeg"].contains(&ext.as_str()) {
        return Err("The photo must be a .png or .jpg file.".to_owned());
    }
    if !p.is_file() {
        return Err(format!("No file at {path}."));
    }
    Ok(())
}

fn load_photo(path: &str) -> Option<slint::Image> {
    if path.is_empty() || check_photo(path).is_err() {
        return None;
    }
    match slint::Image::load_from_path(std::path::Path::new(path)) {
        Ok(i) => Some(i),
        Err(e) => {
            log::warn!("Could not load the profile photo {path}: {e:?}");
            None
        }
    }
}

/// Theme and motion from settings.
pub fn apply_window_settings(w: &AppWindow, s: &Settings) {
    w.global::<Theme>().set_dark(s.theme.is_dark());
    w.global::<Motion>().set_enabled(!s.animation.reduce_motion);
}

fn write_settings(w: &AppWindow, s: &Settings) {
    let v = w.global::<UtilityView>();
    v.set_st_theme(if s.theme.is_dark() { "dark" } else { "light" }.into());
    v.set_st_font(format!("{} px", s.typography.typing_font_px).into());
    v.set_st_bold(s.typography.bold);
    v.set_st_sound(s.sound.enabled);
    v.set_st_reduce_motion(s.animation.reduce_motion);
    v.set_st_backspace(s.backspace().slug().into());
    v.set_st_highlight(s.highlight().slug().into());
    v.set_st_paper(s.typing.paper);
    v.set_st_keyboard(s.typing.key_guide);
    v.set_st_layout(s.keyboard.layout_id().as_str().into());
    v.set_st_guide(s.guide_mode().slug().into());
    v.set_st_focus(s.focus.focus_mode);
}

/// The Home hero (HOME-04): practice time, best net WPM, accuracy, streak,
/// Quick Resume and the 7D / 30D sparkline, all from SQLite.
pub fn refresh_home(w: &AppWindow, db: &Rc<RefCell<radha_db::Database>>, profile_id: i64) {
    let Ok(db) = db.try_borrow() else {
        return;
    };
    let summary = db.summary(profile_id).unwrap_or_else(|e| {
        log::warn!("Home hero: {e}");
        AnalyticsSummary::default()
    });
    let v = w.global::<UtilityView>();
    let days = range_days(v.get_hero_range().as_str());
    let today = today();
    let since = window_start(&today, days).unwrap_or_default();
    let points = in_window(
        &db.daily_points(profile_id, &since).unwrap_or_default(),
        &today,
        days,
    );
    let s = series(&points, days, " WPM", |d| d.average_net_wpm);
    let streak_days = streak(
        &db.practice_days(profile_id, STREAK_DAYS)
            .unwrap_or_default(),
        &today,
    );
    let resume = db
        .recent_sessions(profile_id, 1)
        .ok()
        .and_then(|r| r.into_iter().next())
        .map(|l| {
            format!(
                "Last: {} · Exercise {} · {:.0} net WPM, {:.0}%",
                l.product_name, l.exercise_number, l.net_wpm, l.accuracy
            )
        })
        .unwrap_or_default();
    let has = summary.has_data();
    v.set_hero_has_data(has);
    v.set_hero_practice(
        if has {
            duration_text(summary.total_practice_seconds)
        } else {
            "—".into()
        }
        .into(),
    );
    v.set_hero_best(
        if has {
            format!("{:.0}", summary.best_net_wpm)
        } else {
            "—".into()
        }
        .into(),
    );
    v.set_hero_accuracy(
        if has {
            format!("{:.0}%", summary.average_accuracy)
        } else {
            "—".into()
        }
        .into(),
    );
    v.set_hero_streak(
        match streak_days {
            0 => String::new(),
            1 => "1-day streak · practise today to keep it".to_owned(),
            n => format!("{n}-day practice streak"),
        }
        .into(),
    );
    v.set_hero_resume(resume.into());
    v.set_hero_chart_has_data(s.chart.is_some());
    if let Some(c) = &s.chart {
        v.set_hero_chart_line(c.line.as_str().into());
        v.set_hero_chart_area(c.area.as_str().into());
        v.set_hero_peak_x(c.peak_x);
        v.set_hero_peak_y(c.peak_y);
    }
    v.set_hero_chart_value(s.value.into());
    v.set_hero_chart_footer(s.footer.into());
    v.set_hero_chart_trend(s.trend.into());
}

fn wire(window: &AppWindow, u: &Rc<RefCell<Utility>>) {
    let v = window.global::<UtilityView>();
    let x = Rc::clone(u);
    v.on_open_requested(move |id| {
        if let Ok(mut u) = x.try_borrow_mut() {
            u.open(id.as_str());
        }
    });
    let x = Rc::clone(u);
    v.on_close(move || {
        if let Ok(u) = x.try_borrow() {
            u.close();
        }
    });
    let x = Rc::clone(u);
    let weak = window.as_weak();
    v.on_hero_range_selected(move |id| {
        let id = if id == "30d" { "30d" } else { "7d" };
        if let (Some(w), Ok(u)) = (weak.upgrade(), x.try_borrow()) {
            w.global::<UtilityView>().set_hero_range(id.into());
            refresh_home(&w, &u.db, u.profile_id);
        }
    });
    let x = Rc::clone(u);
    let weak = window.as_weak();
    v.on_analytics_range_selected(move |id| {
        let id = if id == "7d" { "7d" } else { "30d" };
        if let (Some(w), Ok(u)) = (weak.upgrade(), x.try_borrow()) {
            w.global::<UtilityView>().set_an_range(id.into());
            u.fill_analytics();
        }
    });
    let x = Rc::clone(u);
    v.on_resume(move || {
        if let Ok(u) = x.try_borrow() {
            u.resume();
        }
    });
    let x = Rc::clone(u);
    v.on_save_profile(move |name, photo| {
        if let Ok(u) = x.try_borrow() {
            u.save_profile(name.as_str(), photo.as_str());
        }
    });
    let x = Rc::clone(u);
    v.on_set_setting(move |k, val| {
        if let Ok(u) = x.try_borrow() {
            u.set_setting(k.as_str(), val.as_str());
        }
    });
    let x = Rc::clone(u);
    v.on_backup(move |p| {
        if let Ok(u) = x.try_borrow() {
            u.backup(p.as_str());
        }
    });
    let x = Rc::clone(u);
    v.on_export_history(move |p| {
        if let Ok(u) = x.try_borrow() {
            u.export_history(p.as_str());
        }
    });
    v.on_open_link(
        |id| match crate::contact::ContactLink::from_id(id.as_str()) {
            Some(link) => {
                if let Err(e) = crate::contact::open(link) {
                    log::warn!("Could not open the {} link: {e}", link.id());
                }
            }
            None => log::warn!("Ignoring unknown link id '{id}'"),
        },
    );
    let x = Rc::clone(u);
    v.on_save_candidate(move |n, r| {
        if let Ok(u) = x.try_borrow() {
            u.save_candidate(n.as_str(), r.as_str());
        }
    });
    let x = Rc::clone(u);
    v.on_clear_candidate(move || {
        if let Ok(u) = x.try_borrow() {
            u.save_candidate("", "");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: &str, n: u32, net: f64) -> DayPoint {
        DayPoint {
            day: d.into(),
            sessions: n,
            average_net_wpm: net,
            average_accuracy: 95.0,
            ..DayPoint::default()
        }
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_text(0), "—");
        assert_eq!(duration_text(45), "45s");
        assert_eq!(duration_text(720), "12m");
        assert_eq!(duration_text(3900), "1h 05m");
    }

    #[test]
    fn series_are_honest_about_empty_ranges() {
        let empty = series(&[], 7, " WPM", |d| d.average_net_wpm);
        assert!(empty.chart.is_none());
        assert_eq!(empty.footer, "No sessions in the last 7 days");
        assert!(empty.value.is_empty() && empty.trend.is_empty());

        let s = series(
            &[day("2026-10-20", 2, 30.0), day("2026-10-25", 1, 36.0)],
            7,
            " WPM",
            |d| d.average_net_wpm,
        );
        assert!(s.chart.is_some());
        assert_eq!(s.value, "36 WPM latest");
        assert_eq!(s.trend, "+6 WPM vs first day");
        assert_eq!(s.footer, "2 practice days · 3 sessions · last 7 days");
        let one = series(&[day("2026-10-25", 1, 20.0)], 30, " WPM", |d| {
            d.average_net_wpm
        });
        assert_eq!(one.trend, "", "one day has no trend");
        assert_eq!(range_days("30d"), 30);
        assert_eq!(range_days("anything"), 7);
    }

    #[test]
    fn kpis_are_eight_and_from_the_summary() {
        let s = AnalyticsSummary {
            sessions: 4,
            total_practice_seconds: 600,
            average_net_wpm: 31.4,
            best_net_wpm: 40.0,
            average_consistency: None,
            exercises_completed: 2,
            exercises_practised: 3,
            ..AnalyticsSummary::default()
        };
        let k = kpis(&s, 2);
        assert_eq!(k.len(), 8);
        assert_eq!(k[0].1, "4");
        assert_eq!(k[1].1, "10m");
        assert_eq!(k[2].1, "31");
        assert_eq!(k[6].1, "—", "no consistency recorded: no number");
        assert_eq!(k[7].2, "finished · 3 practised");
    }

    #[test]
    fn photos_must_be_existing_images() {
        assert!(check_photo("C:\\nope\\me.gif").is_err());
        assert!(check_photo("C:\\definitely\\missing\\me.png").is_err());
        let temp = tempfile::tempdir().unwrap();
        let p = temp.path().join("me.PNG");
        std::fs::write(&p, b"x").unwrap();
        assert!(check_photo(&p.display().to_string()).is_ok());
        assert!(load_photo("").is_none());
    }
}

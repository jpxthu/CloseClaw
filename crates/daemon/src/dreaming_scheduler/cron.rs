//! Cron expression parsing and next-fire computation for the
//! DreamingScheduler (5-field cron via `dreaming.schedule`).

use chrono::{Datelike, Local, Timelike};

/// Parsed cron schedule (minute, hour, day-of-month, month,
/// day-of-week).
///
/// Supports `*` for "any" and specific numeric values.
#[derive(Debug, Clone)]
pub(crate) struct CronSchedule {
    minute: CronField,
    hour: CronField,
    dom: CronField,
    month: CronField,
    dow: CronField,
}

/// A single cron field — either `*` (any) or a specific numeric value.
#[derive(Debug, Clone)]
enum CronField {
    Any,
    Value(u32),
}

/// Parse a 5-field cron expression into a [`CronSchedule`].
///
/// Only supports `*` and non-negative integer values. Returns `None` on
/// any parse error, allowing the caller to fall back to fixed-interval.
pub(crate) fn parse_cron_schedule(expr: &str) -> Option<CronSchedule> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    Some(CronSchedule {
        minute: parse_cron_field(parts[0], 0, 59)?,
        hour: parse_cron_field(parts[1], 0, 23)?,
        dom: parse_cron_field(parts[2], 1, 31)?,
        month: parse_cron_field(parts[3], 1, 12)?,
        dow: parse_cron_field(parts[4], 0, 6)?,
    })
}

/// Parse a single cron field into a [`CronField`].
fn parse_cron_field(s: &str, min: u32, max: u32) -> Option<CronField> {
    if s == "*" {
        return Some(CronField::Any);
    }
    let val: u32 = s.parse().ok()?;
    if val < min || val > max {
        return None;
    }
    Some(CronField::Value(val))
}

impl CronSchedule {
    /// Compute the next fire time strictly after `now`.
    ///
    /// Iterates minute-by-minute (bounded to avoid infinite loops) to find
    /// the next datetime matching all cron fields.
    fn next_fire_time(&self, now: chrono::DateTime<Local>) -> chrono::DateTime<Local> {
        // Start from the next whole minute after now.
        let mut candidate = now + chrono::Duration::minutes(1);
        candidate = candidate.with_second(0).unwrap_or(candidate);
        candidate = candidate.with_nanosecond(0).unwrap_or(candidate);

        // Cap iterations to prevent infinite loops (max ~2 days of
        // minutes).
        for _ in 0..2880 {
            if self.matches(&candidate) {
                return candidate;
            }
            candidate += chrono::Duration::minutes(1);
        }

        // Fallback: return now + 1 hour (should never reach here
        // for valid cron expressions).
        now + chrono::Duration::hours(1)
    }

    /// Check if a datetime matches all cron fields.
    fn matches(&self, dt: &chrono::DateTime<Local>) -> bool {
        match_field(&self.minute, dt.minute())
            && match_field(&self.hour, dt.hour())
            && match_field(&self.dom, dt.day())
            && match_field(&self.month, dt.month())
            && match_field(&self.dow, dt.weekday().num_days_from_sunday())
    }

    /// Human-readable display for logging.
    pub(crate) fn display(&self) -> String {
        format!(
            "{} {} {} {} {}",
            display_field(&self.minute),
            display_field(&self.hour),
            display_field(&self.dom),
            display_field(&self.month),
            display_field(&self.dow),
        )
    }
}

/// Check if a numeric value matches a [`CronField`].
fn match_field(field: &CronField, value: u32) -> bool {
    match field {
        CronField::Any => true,
        CronField::Value(v) => *v == value,
    }
}

/// Next cron fire instant, expressed on tokio's clock.
///
/// Collapses the three verbatim `next_fire_time` → delay →
/// `tokio::time::Instant` conversions previously duplicated across
/// [`DreamingScheduler::run`](super::DreamingScheduler::run), the cron
/// loop's `Fire` branch and
/// [`DreamingScheduler::handle_cron_config_reload`](super::DreamingScheduler::handle_cron_config_reload)
/// (issue #3578).
/// Falls back to a 1s delay when the wall-clock delta is negative or
/// unrepresentable (clock moved backwards past the fire instant).
pub(crate) fn cron_next_fire(schedule: &CronSchedule) -> tokio::time::Instant {
    let next = schedule.next_fire_time(Local::now());
    let delay = (next - Local::now())
        .to_std()
        .unwrap_or_else(|_| tokio::time::Duration::from_secs(1));
    tokio::time::Instant::now() + delay
}

/// Display a cron field for logging.
fn display_field(field: &CronField) -> String {
    match field {
        CronField::Any => "*".to_string(),
        CronField::Value(v) => v.to_string(),
    }
}

#[cfg(test)]
mod cron_tests {
    use super::*;

    #[test]
    fn test_parse_cron_valid() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        assert!(matches!(sched.minute, CronField::Value(0)));
        assert!(matches!(sched.hour, CronField::Value(3)));
        assert!(matches!(sched.dom, CronField::Any));
        assert!(matches!(sched.month, CronField::Any));
        assert!(matches!(sched.dow, CronField::Any));
    }

    #[test]
    fn test_parse_cron_invalid_parts() {
        assert!(parse_cron_schedule("0 3 * *").is_none());
        assert!(parse_cron_schedule("0 3 * * * *").is_none());
        assert!(parse_cron_schedule("").is_none());
    }

    #[test]
    fn test_parse_cron_invalid_values() {
        assert!(parse_cron_schedule("60 3 * * *").is_none());
        assert!(parse_cron_schedule("0 24 * * *").is_none());
        assert!(parse_cron_schedule("0 3 0 * *").is_none());
        assert!(parse_cron_schedule("0 3 * 13 *").is_none());
    }

    #[test]
    fn test_parse_cron_non_numeric() {
        assert!(parse_cron_schedule("abc 3 * * *").is_none());
    }

    #[test]
    fn test_next_fire_daily_at_3am() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        // 2024-07-06 10:00 — next fire should be 2024-07-07 03:00
        let now = chrono::NaiveDate::from_ymd_opt(2024, 7, 6)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        let next = sched.next_fire_time(now);
        assert_eq!(next.hour(), 3);
        assert_eq!(next.minute(), 0);
        assert!(next > now);
    }

    #[test]
    fn test_next_fire_before_3am() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        // 2024-07-06 02:00 — next fire should be same day 03:00
        let now = chrono::NaiveDate::from_ymd_opt(2024, 7, 6)
            .unwrap()
            .and_hms_opt(2, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        let next = sched.next_fire_time(now);
        assert_eq!(next.hour(), 3);
        assert_eq!(next.minute(), 0);
        assert_eq!(next.date_naive(), now.date_naive());
    }

    #[test]
    fn test_next_fire_exact_3am() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        // At exactly 3:00 → next should be tomorrow 3:00
        let now = chrono::NaiveDate::from_ymd_opt(2024, 7, 6)
            .unwrap()
            .and_hms_opt(3, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        let next = sched.next_fire_time(now);
        assert_eq!(next.hour(), 3);
        assert_eq!(next.minute(), 0);
        assert!(next > now);
    }

    #[test]
    fn test_matches_star_matches_all() {
        let field = CronField::Any;
        assert!(match_field(&field, 0));
        assert!(match_field(&field, 59));
    }

    #[test]
    fn test_matches_value_only_exact() {
        let field = CronField::Value(42);
        assert!(match_field(&field, 42));
        assert!(!match_field(&field, 43));
        assert!(!match_field(&field, 41));
    }

    #[test]
    fn test_display_star() {
        assert_eq!(display_field(&CronField::Any), "*");
    }

    #[test]
    fn test_display_value() {
        assert_eq!(display_field(&CronField::Value(5)), "5");
    }

    #[test]
    fn test_schedule_display() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        assert_eq!(sched.display(), "0 3 * * *");
    }
}

use sim_kernel::{Error, Result};

use crate::WallTimestamp;

#[derive(Clone)]
pub(crate) struct CronMatcher {
    minute: FieldMatcher,
    hour: FieldMatcher,
    day: FieldMatcher,
    month: FieldMatcher,
    weekday: FieldMatcher,
}

impl CronMatcher {
    pub(crate) fn parse(spec: &str) -> Result<Self> {
        let parts = spec.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 5 {
            return Err(Error::Eval(format!("cron spec {spec} must have 5 fields")));
        }
        Ok(Self {
            minute: FieldMatcher::parse(parts[0], 0, 59, "minute")?,
            hour: FieldMatcher::parse(parts[1], 0, 23, "hour")?,
            day: FieldMatcher::parse(parts[2], 1, 31, "day")?,
            month: FieldMatcher::parse(parts[3], 1, 12, "month")?,
            weekday: FieldMatcher::parse(parts[4], 0, 6, "weekday")?,
        })
    }

    pub(crate) fn matches_fields(
        &self,
        minute: u8,
        hour: u8,
        day: u8,
        month: u8,
        weekday: u8,
    ) -> bool {
        self.minute.matches(minute)
            && self.hour.matches(hour)
            && self.day.matches(day)
            && self.month.matches(month)
            && self.weekday.matches(weekday)
    }

    pub(crate) fn current_match(&self, now: WallTimestamp) -> Option<u64> {
        let minute_key = now.unix_millis() / 60_000;
        let day_number = minute_key / (24 * 60);
        let minute_of_day = (minute_key % (24 * 60)) as u16;
        let hour = (minute_of_day / 60) as u8;
        let minute = (minute_of_day % 60) as u8;
        let (year, month, day) = civil_from_days(day_number as i64);
        let _ = year;
        let weekday = ((day_number + 4) % 7) as u8;
        self.matches_fields(minute, hour, day as u8, month as u8, weekday)
            .then_some(minute_key)
    }
}

#[derive(Clone)]
struct FieldMatcher {
    allowed: [bool; 64],
}

impl FieldMatcher {
    fn parse(spec: &str, min: u8, max: u8, label: &str) -> Result<Self> {
        let mut allowed = [false; 64];
        for part in spec.split(',') {
            if part == "*" {
                for value in min..=max {
                    allowed[value as usize] = true;
                }
                continue;
            }
            if let Some(step) = part.strip_prefix("*/") {
                let step = parse_u8(step, label)?;
                if step == 0 {
                    return Err(Error::Eval(format!("cron {label} step must be > 0")));
                }
                let mut value = min;
                while value <= max {
                    allowed[value as usize] = true;
                    match value.checked_add(step) {
                        Some(next) if next > value => value = next,
                        _ => break,
                    }
                }
                continue;
            }
            if let Some((start, end)) = part.split_once('-') {
                let start = parse_u8(start, label)?;
                let end = parse_u8(end, label)?;
                if start < min || end > max || start > end {
                    return Err(Error::Eval(format!("cron {label} range {part} is invalid")));
                }
                for value in start..=end {
                    allowed[value as usize] = true;
                }
                continue;
            }
            let value = parse_u8(part, label)?;
            if value < min || value > max {
                return Err(Error::Eval(format!(
                    "cron {label} value {value} is out of range {min}..={max}"
                )));
            }
            allowed[value as usize] = true;
        }
        Ok(Self { allowed })
    }

    fn matches(&self, value: u8) -> bool {
        self.allowed[value as usize]
    }
}

fn parse_u8(text: &str, label: &str) -> Result<u8> {
    text.parse::<u8>()
        .map_err(|_| Error::Eval(format!("cron {label} token {text} is not a number")))
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::CronMatcher;

    #[test]
    fn cron_matcher_supports_every_n_minutes() {
        let matcher = CronMatcher::parse("*/5 * * * *").unwrap();
        assert!(matcher.matches_fields(0, 3, 10, 6, 1));
        assert!(matcher.matches_fields(55, 3, 10, 6, 1));
        assert!(!matcher.matches_fields(7, 3, 10, 6, 1));
    }

    #[test]
    fn cron_matcher_supports_ranges() {
        let matcher = CronMatcher::parse("0 9-17 * * *").unwrap();
        assert!(matcher.matches_fields(0, 9, 1, 1, 0));
        assert!(matcher.matches_fields(0, 17, 1, 1, 0));
        assert!(!matcher.matches_fields(0, 18, 1, 1, 0));
    }

    #[test]
    fn cron_matcher_supports_lists() {
        let matcher = CronMatcher::parse("15 8,12,18 * * 1,3,5").unwrap();
        assert!(matcher.matches_fields(15, 12, 1, 1, 3));
        assert!(!matcher.matches_fields(15, 11, 1, 1, 3));
        assert!(!matcher.matches_fields(15, 12, 1, 1, 2));
    }
}

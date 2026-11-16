//! English dates and the calendar grid. Does not depend on system locales.

use chrono::{Datelike, Months, NaiveDate};

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
/// Column headers, Monday first.
pub const WEEKDAYS_SHORT: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
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

/// Weeks the grid always shows, so the panel keeps its height across months.
pub const WEEKS: usize = 6;

/// "Thursday, November 12".
pub fn long_date(date: NaiveDate) -> String {
    let weekday = WEEKDAYS[date.weekday().num_days_from_monday() as usize];
    format!("{weekday}, {} {}", month_name(date), date.day())
}

/// "November 2026".
pub fn month_title(month: NaiveDate) -> String {
    format!("{} {}", month_name(month), month.year())
}

fn month_name(date: NaiveDate) -> &'static str {
    MONTHS[date.month0() as usize]
}

/// First day of the month of `date`.
pub fn first_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("day 1 always exists")
}

/// First day of the month shifted by `delta` months.
pub fn shift_month(month: NaiveDate, delta: i32) -> NaiveDate {
    let month = first_of_month(month);
    let months = Months::new(delta.unsigned_abs());
    let shifted = if delta >= 0 {
        month.checked_add_months(months)
    } else {
        month.checked_sub_months(months)
    };
    shifted.unwrap_or(month)
}

/// Monday–Sunday grid of `WEEKS` weeks; `None` in cells outside the month.
pub fn month_grid(month: NaiveDate) -> [[Option<u32>; 7]; WEEKS] {
    let first = first_of_month(month);
    let offset = first.weekday().num_days_from_monday() as usize;
    let days = shift_month(first, 1)
        .pred_opt()
        .map_or(28, |last| last.day()) as usize;
    let mut grid = [[None; 7]; WEEKS];
    for day in 0..days {
        let cell = offset + day;
        if let Some(week) = grid.get_mut(cell / 7) {
            week[cell % 7] = Some(day as u32 + 1);
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn formats_english_dates() {
        assert_eq!(long_date(date(2026, 11, 12)), "Thursday, November 12");
        assert_eq!(long_date(date(2026, 3, 1)), "Sunday, March 1");
        assert_eq!(long_date(date(2025, 12, 31)), "Wednesday, December 31");
        assert_eq!(month_title(date(2026, 11, 12)), "November 2026");
    }

    #[test]
    fn shifts_months_across_years() {
        assert_eq!(shift_month(date(2026, 11, 30), 1), date(2026, 12, 1));
        assert_eq!(shift_month(date(2026, 12, 15), 1), date(2027, 1, 1));
        assert_eq!(shift_month(date(2026, 1, 31), -1), date(2025, 12, 1));
    }

    #[test]
    fn builds_month_grid() {
        // November 2026 starts on a Sunday and has 30 days.
        let grid = month_grid(date(2026, 11, 12));
        assert_eq!(grid[0], [None, None, None, None, None, None, Some(1)]);
        assert_eq!(grid[1][0], Some(2));
        assert_eq!(grid[5][0], Some(30));
        assert_eq!(grid[5][1], None);
        // February 2027 starts on a Monday, 28 days: four full weeks and two empty ones.
        let grid = month_grid(date(2027, 2, 1));
        assert_eq!(grid[0][0], Some(1));
        assert_eq!(grid[3][6], Some(28));
        assert!(grid[4].iter().all(Option::is_none));
        // Leap-year February.
        let days: usize = month_grid(date(2028, 2, 1))
            .iter()
            .flatten()
            .filter(|cell| cell.is_some())
            .count();
        assert_eq!(days, 29);
    }
}

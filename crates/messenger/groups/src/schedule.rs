// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! When a group is open for posting: a window of hours on days of the
//! week, read on the clock of one time zone. The zone is named in the
//! settings and its rules come with the app, so that every device opens
//! and closes the group at the same moment.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp};
use serde::{Deserialize, Serialize};

/// The hours a group takes messages from its members: `from` to `to`
/// (`HH:MM`, 24 hours) on each of `days` (1 = Monday … 7 = Sunday) in the
/// zone `tz`. A window whose `from` is later than its `to` runs over
/// midnight into the next day; it is counted on the day it starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub tz: String,
    pub days: Vec<u8>,
    pub from: String,
    pub to: String,
}

pub const MAX_TZ_CHARS: usize = 64;

/// The zone is not in the rules this build carries: it is newer than
/// the app, or not a zone at all. The schedule is read in UTC then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownTz;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Open,
    /// Closed, and the moment (unix seconds) the next window starts.
    Closed { opens_at: i64 },
}

impl Schedule {
    /// Whether the schedule is one the log takes: a zone name of the
    /// shape names have (whether the zone exists is not checked here, or
    /// a device with older rules would refuse what a newer one accepted),
    /// days in ascending order without repeats, two different times.
    /// Empty `days` are not a schedule: an edit uses them to take the
    /// schedule away.
    pub fn check(&self) -> Result<(), &'static str> {
        let tz = self.tz.as_str();
        if tz.is_empty() || tz.len() > MAX_TZ_CHARS || !tz.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'+')) {
            return Err("tz");
        }
        if self.days.len() > 7 || self.days.windows(2).any(|w| w[0] >= w[1]) || self.days.iter().any(|d| !(1..=7).contains(d)) {
            return Err("days");
        }
        let (Some(from), Some(to)) = (minute_of_day(&self.from), minute_of_day(&self.to)) else { return Err("time") };
        if from == to {
            return Err("time");
        }
        Ok(())
    }

    /// A schedule with no days is no schedule.
    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }
}

/// `HH:MM` as minutes since midnight.
fn minute_of_day(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Is the group open at `now` (unix seconds) by this schedule, and if
/// not, when does it open? `UnknownTz` when the zone is not known here.
pub fn status(s: &Schedule, now: i64) -> Result<Status, UnknownTz> {
    let tz = TimeZone::get(&s.tz).map_err(|_| UnknownTz)?;
    Ok(status_in(s, &tz, now))
}

/// `status`, in UTC when the zone is unknown; whether it was.
pub fn status_or_utc(s: &Schedule, now: i64) -> (Status, bool) {
    match status(s, now) {
        Ok(st) => (st, false),
        Err(UnknownTz) => (status_in(s, &TimeZone::UTC, now), true),
    }
}

pub fn open_at(s: &Schedule, now: i64) -> Result<bool, UnknownTz> {
    status(s, now).map(|st| st == Status::Open)
}

fn status_in(s: &Schedule, tz: &TimeZone, now: i64) -> Status {
    let (Some(from), Some(to)) = (minute_of_day(&s.from), minute_of_day(&s.to)) else { return Status::Open };
    if s.days.is_empty() {
        return Status::Open;
    }
    let Ok(ts) = Timestamp::from_second(now) else { return Status::Open };
    let today = ts.to_zoned(tz.clone()).date();
    // The window that started yesterday may still be open; after that,
    // today's and those of the week ahead, in order: the first one not
    // over yet is the answer.
    for offset in -1..=7i64 {
        let Ok(day) = today.checked_add(Span::new().days(offset)) else { continue };
        if !s.days.contains(&(day.weekday().to_monday_one_offset() as u8)) {
            continue;
        }
        let Some(start) = moment(day, from, tz) else { continue };
        let end_day = if to <= from { day.tomorrow().ok() } else { Some(day) };
        let Some(end) = end_day.and_then(|d| moment(d, to, tz)) else { continue };
        if now < start {
            return Status::Closed { opens_at: start };
        }
        if now < end {
            return Status::Open;
        }
    }
    Status::Open
}

/// Unix seconds of a wall-clock minute of a day in a zone. A minute the
/// clocks skip (the spring change) is taken as the one after the gap.
fn moment(day: Date, minute: u32, tz: &TimeZone) -> Option<i64> {
    day.at((minute / 60) as i8, (minute % 60) as i8, 0, 0).to_zoned(tz.clone()).ok().map(|z| z.timestamp().as_second())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(tz: &str, y: i16, mo: i8, d: i8, h: i8, mi: i8) -> i64 {
        jiff::civil::date(y, mo, d).at(h, mi, 0, 0).to_zoned(TimeZone::get(tz).unwrap()).unwrap().timestamp().as_second()
    }

    fn sched(tz: &str, days: &[u8], from: &str, to: &str) -> Schedule {
        Schedule { tz: tz.into(), days: days.to_vec(), from: from.into(), to: to.into() }
    }

    #[test]
    fn shape() {
        assert_eq!(sched("Europe/Berlin", &[1, 2, 3], "09:00", "18:00").check(), Ok(()));
        assert_eq!(sched("Etc/GMT+3", &[7], "23:00", "01:00").check(), Ok(()));
        assert_eq!(sched("", &[1], "09:00", "18:00").check(), Err("tz"));
        assert_eq!(sched("Europe/Berlin; drop", &[1], "09:00", "18:00").check(), Err("tz"));
        assert_eq!(sched("Mars/Olympus", &[1], "09:00", "18:00").check(), Ok(()), "an unknown zone is taken: another device may know it");
        assert_eq!(sched("UTC", &[0], "09:00", "18:00").check(), Err("days"));
        assert_eq!(sched("UTC", &[1, 1], "09:00", "18:00").check(), Err("days"));
        assert_eq!(sched("UTC", &[2, 1], "09:00", "18:00").check(), Err("days"));
        assert_eq!(sched("UTC", &[1, 2, 3, 4, 5, 6, 7, 7], "09:00", "18:00").check(), Err("days"));
        assert_eq!(sched("UTC", &[], "09:00", "18:00").check(), Ok(()), "no days: the edit takes the schedule away");
        assert!(sched("UTC", &[], "09:00", "18:00").is_empty());
        assert_eq!(sched("UTC", &[1], "9:00", "18:00").check(), Err("time"));
        assert_eq!(sched("UTC", &[1], "24:00", "18:00").check(), Err("time"));
        assert_eq!(sched("UTC", &[1], "09:60", "18:00").check(), Err("time"));
        assert_eq!(sched("UTC", &[1], "09:00", "09:00").check(), Err("time"));
    }

    #[test]
    fn a_window_in_a_zone_through_the_spring_change() {
        // Berlin moves its clocks forward on Sunday 2026-03-29 at 02:00.
        let s = sched("Europe/Berlin", &[6, 7], "09:00", "18:00");
        let sat = at("Europe/Berlin", 2026, 3, 28, 12, 0);
        assert_eq!(status(&s, sat), Ok(Status::Open));
        let sat_night = at("Europe/Berlin", 2026, 3, 28, 23, 0);
        let sun_nine = at("Europe/Berlin", 2026, 3, 29, 9, 0);
        assert_eq!(status(&s, sat_night), Ok(Status::Closed { opens_at: sun_nine }));
        assert_eq!(sun_nine - sat_night, 9 * 3600, "one hour less: the night was short");
        assert_eq!(status(&s, sun_nine - 1), Ok(Status::Closed { opens_at: sun_nine }));
        assert_eq!(status(&s, sun_nine), Ok(Status::Open));
        assert_eq!(status(&s, at("Europe/Berlin", 2026, 3, 29, 17, 59)), Ok(Status::Open));
        // Closed from 18:00 Sunday until 09:00 next Saturday.
        let next_sat = at("Europe/Berlin", 2026, 4, 4, 9, 0);
        assert_eq!(status(&s, at("Europe/Berlin", 2026, 3, 29, 18, 0)), Ok(Status::Closed { opens_at: next_sat }));
        assert_eq!(status(&s, at("Europe/Berlin", 2026, 4, 1, 12, 0)), Ok(Status::Closed { opens_at: next_sat }));
        // A window that starts in the skipped hour starts right after it.
        let gap = sched("Europe/Berlin", &[7], "02:30", "04:00");
        assert_eq!(status(&gap, at("Europe/Berlin", 2026, 3, 29, 1, 0)), Ok(Status::Closed { opens_at: at("Europe/Berlin", 2026, 3, 29, 3, 30) }));
        // The same wall-clock window means another moment in another zone.
        let tokyo = sched("Asia/Tokyo", &[6, 7], "09:00", "18:00");
        assert_eq!(status(&tokyo, sat_night), Ok(Status::Closed { opens_at: at("Asia/Tokyo", 2026, 3, 29, 9, 0) }));
    }

    /// Berlin sets its clocks back on Sunday 2026-10-25 at 03:00: the
    /// hour from 02:00 to 03:00 comes twice. A wall-clock time in it is
    /// taken at its first coming (the offset before the change), the same
    /// on every device, so a window that starts in that hour starts once
    /// and runs through the repeated hour to its end.
    #[test]
    fn a_window_in_a_zone_through_the_autumn_change() {
        let s = sched("Europe/Berlin", &[7], "02:30", "04:00");
        // 02:30 CEST is 00:30 UTC; 04:00 CET is 03:00 UTC.
        let start = at("UTC", 2026, 10, 25, 0, 30);
        let end = at("UTC", 2026, 10, 25, 3, 0);
        assert_eq!(status(&s, start - 1), Ok(Status::Closed { opens_at: start }));
        assert_eq!(status(&s, at("UTC", 2026, 10, 24, 12, 0)), Ok(Status::Closed { opens_at: start }));
        assert_eq!(status(&s, start), Ok(Status::Open));
        assert_eq!(status(&s, at("UTC", 2026, 10, 25, 1, 30)), Ok(Status::Open), "02:30 the second time: still inside");
        assert_eq!(status(&s, end - 1), Ok(Status::Open));
        assert_eq!(status(&s, end), Ok(Status::Closed { opens_at: at("UTC", 2026, 11, 1, 1, 30) }), "next Sunday, now in winter time");
    }

    #[test]
    fn an_overnight_window_and_the_days_it_counts_on() {
        // Friday night into Saturday morning: counted on Friday.
        let s = sched("UTC", &[5], "22:00", "02:00");
        assert_eq!(status(&s, at("UTC", 2026, 10, 16, 21, 59)), Ok(Status::Closed { opens_at: at("UTC", 2026, 10, 16, 22, 0) }));
        assert_eq!(status(&s, at("UTC", 2026, 10, 16, 22, 0)), Ok(Status::Open));
        assert_eq!(status(&s, at("UTC", 2026, 10, 17, 1, 59)), Ok(Status::Open), "past midnight, still Friday's window");
        assert_eq!(status(&s, at("UTC", 2026, 10, 17, 2, 0)), Ok(Status::Closed { opens_at: at("UTC", 2026, 10, 23, 22, 0) }));
        // Saturday 22:00 is not a window: the day is not listed.
        assert_eq!(status(&s, at("UTC", 2026, 10, 17, 23, 0)), Ok(Status::Closed { opens_at: at("UTC", 2026, 10, 23, 22, 0) }));
        // Every day, all but one minute: closed for that minute only.
        let s = sched("UTC", &[1, 2, 3, 4, 5, 6, 7], "00:01", "00:00");
        assert_eq!(status(&s, at("UTC", 2026, 10, 16, 0, 0)), Ok(Status::Closed { opens_at: at("UTC", 2026, 10, 16, 0, 1) }));
        assert_eq!(status(&s, at("UTC", 2026, 10, 16, 23, 59)), Ok(Status::Open));
        assert_eq!(open_at(&s, at("UTC", 2026, 10, 16, 12, 0)), Ok(true));
    }

    #[test]
    fn an_unknown_zone_is_read_in_utc_and_said_so() {
        let s = sched("Mars/Olympus", &[5], "09:00", "18:00");
        assert_eq!(status(&s, 0), Err(UnknownTz));
        assert_eq!(open_at(&s, 0), Err(UnknownTz));
        let noon = at("UTC", 2026, 10, 16, 12, 0);
        assert_eq!(status_or_utc(&s, noon), (Status::Open, true));
        assert_eq!(status_or_utc(&sched("UTC", &[5], "09:00", "18:00"), noon), (Status::Open, false));
        assert_eq!(status_or_utc(&s, at("UTC", 2026, 10, 16, 8, 0)), (Status::Closed { opens_at: at("UTC", 2026, 10, 16, 9, 0) }, true));
        // No days: always open, whatever the zone.
        assert_eq!(status(&sched("UTC", &[], "09:00", "18:00"), noon), Ok(Status::Open));
    }
}

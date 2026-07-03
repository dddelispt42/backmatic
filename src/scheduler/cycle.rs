use chrono::{DateTime, Duration, Local};

use crate::inject::Clock;
use crate::shutdown::Shutdown;

pub struct CycleScheduler {
    anchor: DateTime<Local>,
    interval: Duration,
    cycle_index: u64,
}

impl CycleScheduler {
    pub fn new(clock: &dyn Clock, interval_hours: u64) -> Self {
        Self {
            anchor: clock.now(),
            interval: Duration::hours(interval_hours as i64),
            cycle_index: 1,
        }
    }

    pub fn next_deadline(&self) -> DateTime<Local> {
        self.anchor + self.interval * self.cycle_index as i32
    }

    pub fn sleep_until_next(&self, clock: &dyn Clock, shutdown: &Shutdown) -> bool {
        let now = clock.now();
        let next = self.next_deadline();
        if next > now {
            let total_secs = (next - now).num_seconds().max(0) as u64;
            log::info!("Next cycle at {next} (sleeping {total_secs}s)");
            let mut remaining = total_secs;
            while remaining > 0 && !shutdown.is_requested() {
                let chunk = remaining.min(1);
                clock.sleep(std::time::Duration::from_secs(chunk));
                remaining = remaining.saturating_sub(chunk);
            }
            false
        } else {
            log::warn!("Cycle overrun: scheduled {next}, now {now}");
            true
        }
    }

    pub fn advance(&mut self) {
        self.cycle_index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inject::clock::FakeClock;
    use chrono::{Datelike, NaiveDate, NaiveTime, TimeZone, Timelike};

    #[test]
    fn next_deadline_is_anchor_plus_interval() {
        let naive = NaiveDate::from_ymd_opt(2026, 6, 29)
            .unwrap()
            .and_time(NaiveTime::from_hms_opt(14, 30, 0).unwrap());
        let start = Local.from_local_datetime(&naive).unwrap();
        let clock = FakeClock::new(start);
        let sched = CycleScheduler::new(&clock, 24);
        let next = sched.next_deadline();
        assert_eq!(next.day(), 30);
        assert_eq!(next.hour(), 14);
    }
}

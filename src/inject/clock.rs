use chrono::{DateTime, Local};
use std::time::{Duration, Instant};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Local>;
    fn sleep(&self, duration: Duration);
    fn instant(&self) -> Instant;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Local> {
        Local::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn instant(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug, Clone)]
pub struct FakeClock {
    now: DateTime<Local>,
}

impl FakeClock {
    pub fn new(at: DateTime<Local>) -> Self {
        Self { now: at }
    }

    pub fn advance(&mut self, duration: chrono::Duration) {
        self.now = self.now + duration;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Local> {
        self.now
    }

    fn sleep(&self, _duration: Duration) {}

    fn instant(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, NaiveDate, NaiveTime, TimeZone};

    #[test]
    fn fake_clock_advances() {
        let naive = NaiveDate::from_ymd_opt(2026, 6, 29)
            .unwrap()
            .and_time(NaiveTime::from_hms_opt(14, 30, 0).unwrap());
        let start = Local.from_local_datetime(&naive).unwrap();
        let mut clock = FakeClock::new(start);
        clock.advance(chrono::Duration::hours(24));
        assert_eq!(clock.now().day(), 30);
    }
}

use chrono::TimeDelta;
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssTime(TimeDelta);

impl AssTime {
    // 从厘秒（百分之一秒）创建时间戳。
    pub fn from_centiseconds(cs: i64) -> Self {
        Self(TimeDelta::milliseconds(cs * 10))
    }

    // 从时、分、秒、厘秒创建时间戳。
    pub fn from_hms_cs(hours: i64, minutes: i64, seconds: i64, centiseconds: i64) -> Self {
        let total_cs = hours * 360000 + minutes * 6000 + seconds * 100 + centiseconds;
        Self::from_centiseconds(total_cs)
    }

    // 获取总厘秒数。
    pub fn as_centiseconds(&self) -> i64 {
        self.0.num_milliseconds() / 10
    }

    pub fn subsec_centis(&self) -> i32 {
        self.0.subsec_millis() / 10
    }
}
impl FromStr for AssTime {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(":").collect();
        if parts.len() != 3 {
            return Err(format!("Invalid time format: {s}"));
        }

        let hours = parts[0]
            .parse::<i64>()
            .map_err(|_| format!("Invalid hours in time: {s}"))?;
        let minutes = parts[1]
            .parse::<i64>()
            .map_err(|_| format!("Invalid minutes in time: {s}"))?;

        let sec_parts: Vec<&str> = parts[2].split(".").collect();
        if sec_parts.len() != 2 {
            return Err(format!("Invalid seconds and centiseconds in time: {s}"));
        }

        let seconds = sec_parts[0]
            .parse::<i64>()
            .map_err(|_| format!("Invalid seconds in time: {s}"))?;
        let centiseconds = sec_parts[1]
            .parse::<i64>()
            .map_err(|_| format!("Invalid centiseconds in time: {s}"))?;

        Ok(AssTime::from_hms_cs(hours, minutes, seconds, centiseconds))
    }
}

impl Display for AssTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 整秒数
        let total_seconds = self.0.num_seconds();

        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        // 小于一秒的厘秒数
        let centiseconds = self.subsec_centis();

        write!(
            f,
            "{}:{:02}:{:02}.{:02}",
            hours, minutes, seconds, centiseconds
        )
    }
}

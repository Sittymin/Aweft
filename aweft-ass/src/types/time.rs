use crate::error::TimeParseError;
use chrono::TimeDelta;
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;

/// ASS 字幕时间戳
///
/// 内部使用 `chrono::TimeDelta` 存储，支持负数时间（libass 特性）。
/// 时间格式为 `H:MM:SS.CC`，其中 CC 是厘秒（百分之一秒）。
///
/// # Examples
///
/// ```
/// use aweft_ass::types::time::AssTime;
/// use std::str::FromStr;
///
/// let time = AssTime::from_str("0:01:23.45").unwrap();
/// assert_eq!(time.as_centiseconds(), 8345);
///
/// let time2 = AssTime::from_hms_cs(0, 1, 23, 45);
/// assert_eq!(time, time2);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssTime(TimeDelta);

impl AssTime {
    /// 从厘秒（百分之一秒）创建时间戳
    ///
    /// # Examples
    ///
    /// ```
    /// use aweft_ass::types::time::AssTime;
    ///
    /// let time = AssTime::from_centiseconds(100); // 1 秒
    /// assert_eq!(time.as_centiseconds(), 100);
    /// ```
    pub fn from_centiseconds(cs: i64) -> Self {
        Self(TimeDelta::milliseconds(cs * 10))
    }

    /// 从时、分、秒、厘秒创建时间戳
    ///
    /// # Examples
    ///
    /// ```
    /// use aweft_ass::types::time::AssTime;
    ///
    /// let time = AssTime::from_hms_cs(1, 30, 45, 50);
    /// // 1小时 + 30分钟 + 45秒 + 50厘秒
    /// assert_eq!(time.as_centiseconds(), 544550);
    /// ```
    pub fn from_hms_cs(hours: i64, minutes: i64, seconds: i64, centiseconds: i64) -> Self {
        let total_cs = hours * 360000 + minutes * 6000 + seconds * 100 + centiseconds;
        Self::from_centiseconds(total_cs)
    }

    /// 获取总厘秒数
    pub fn as_centiseconds(&self) -> i64 {
        self.0.num_milliseconds() / 10
    }

    /// 获取不足一秒的厘秒部分
    pub fn subsec_centis(&self) -> i32 {
        self.0.subsec_millis() / 10
    }
}

impl FromStr for AssTime {
    type Err = TimeParseError;

    /// 从字符串解析时间戳
    ///
    /// 期望格式：`H:MM:SS.CC` 或 `-H:MM:SS.CC`（负数时间）
    ///
    /// # Errors
    ///
    /// 返回 `TimeParseError` 如果格式不正确或数值无效
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return Err(TimeParseError::InvalidFormat {
                input: s.to_string(),
            });
        }

        let hours = parts[0]
            .parse::<i64>()
            .map_err(|_| TimeParseError::InvalidHours {
                value: parts[0].to_string(),
            })?;

        let minutes = parts[1]
            .parse::<i64>()
            .map_err(|_| TimeParseError::InvalidMinutes {
                value: parts[1].to_string(),
            })?;

        let sec_parts: Vec<&str> = parts[2].split('.').collect();
        if sec_parts.len() != 2 {
            return Err(TimeParseError::InvalidFormat {
                input: s.to_string(),
            });
        }

        let seconds = sec_parts[0]
            .parse::<i64>()
            .map_err(|_| TimeParseError::InvalidSeconds {
                value: sec_parts[0].to_string(),
            })?;

        let centiseconds =
            sec_parts[1]
                .parse::<i64>()
                .map_err(|_| TimeParseError::InvalidCentiseconds {
                    value: sec_parts[1].to_string(),
                })?;

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

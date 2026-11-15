use std::fmt;

use serde::Deserialize;
use serde::de::Error;

/// A time of the night, in minutes since the midnight before it; files write it `"20:40"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameTime(u16);

impl GameTime {
    /// The midnight that ends the night, written `"24:00"`.
    pub const MIDNIGHT: GameTime = GameTime(24 * 60);

    pub fn from_hm(hour: u16, minute: u16) -> Option<Self> {
        let valid = minute < 60 && (hour < 24 || (hour == 24 && minute == 0));
        valid.then(|| Self(hour * 60 + minute))
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (hour, minute) = text.split_once(':')?;
        let two_digits = |part: &str| part.len() == 2 && part.bytes().all(|b| b.is_ascii_digit());
        if !two_digits(hour) || !two_digits(minute) {
            return None;
        }
        Self::from_hm(hour.parse().ok()?, minute.parse().ok()?)
    }

    pub fn minutes(self) -> u16 {
        self.0
    }

    pub fn hour(self) -> u16 {
        self.0 / 60
    }

    pub fn minute(self) -> u16 {
        self.0 % 60
    }
}

impl fmt::Display for GameTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour(), self.minute())
    }
}

impl<'de> Deserialize<'de> for GameTime {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        GameTime::parse(&text).ok_or_else(|| D::Error::custom("a time is written \"HH:MM\""))
    }
}

/// A span of the night, from `from` up to but not including `until`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "WindowFields")]
pub struct TimeWindow {
    pub from: GameTime,
    pub until: GameTime,
}

#[derive(Deserialize)]
#[serde(rename = "TimeWindow", deny_unknown_fields)]
struct WindowFields {
    from: GameTime,
    until: GameTime,
}

impl TryFrom<WindowFields> for TimeWindow {
    type Error = &'static str;

    fn try_from(fields: WindowFields) -> Result<Self, Self::Error> {
        if fields.from < fields.until {
            Ok(Self {
                from: fields.from,
                until: fields.until,
            })
        } else {
            Err("a window ends after it starts")
        }
    }
}

impl TimeWindow {
    pub fn contains(&self, time: GameTime) -> bool {
        self.from <= time && time < self.until
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_parses_from_hh_mm_up_to_midnight_and_displays_as_it_is_written() {
        for text in ["00:00", "08:05", "20:40", "24:00"] {
            assert_eq!(GameTime::parse(text).unwrap().to_string(), text);
        }
        assert_eq!(GameTime::parse("20:40").unwrap().minutes(), 20 * 60 + 40);
        assert_eq!(GameTime::parse("24:00"), Some(GameTime::MIDNIGHT));
        for text in [
            "24:01", "25:00", "19:60", "7:05", "19:5", "1930", "19:30:00", "+9:30", "",
        ] {
            assert_eq!(GameTime::parse(text), None, "{text}");
        }
    }
}

//! Shared bounded timestamp parsing for provider record envelopes.

use serde_json::Value;

pub fn parse_timestamp_value(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        return Some(if number > 10_000_000_000 {
            number / 1000
        } else {
            number
        });
    }
    if let Some(number) = value.as_i64() {
        return (number >= 0).then_some(number as u64).map(|value| {
            if value > 10_000_000_000 {
                value / 1000
            } else {
                value
            }
        });
    }
    value.as_str().and_then(parse_rfc3339)
}

pub fn parse_rfc3339(value: &str) -> Option<u64> {
    let (date, rest) = value.split_once('T')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i64>().ok()?;
    let month = date_parts.next()?.parse::<u32>().ok()?;
    let day = date_parts.next()?.parse::<u32>().ok()?;
    let zone_index = rest
        .char_indices()
        .skip(1)
        .find(|(_, character)| matches!(character, 'Z' | '+' | '-'))
        .map(|(index, _)| index)?;
    let (clock, zone) = rest.split_at(zone_index);
    let mut clock_parts = clock.split(':');
    let hour = clock_parts.next()?.parse::<u32>().ok()?;
    let minute = clock_parts.next()?.parse::<u32>().ok()?;
    let second = clock_parts.next()?.split('.').next()?.parse::<u32>().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let offset = if zone == "Z" {
        0i64
    } else {
        let sign = if zone.starts_with('+') {
            1
        } else if zone.starts_with('-') {
            -1
        } else {
            return None;
        };
        let mut parts = zone[1..].split(':');
        let hours = parts.next()?.parse::<i64>().ok()?;
        let minutes = parts.next()?.parse::<i64>().ok()?;
        if hours > 23 || minutes > 59 {
            return None;
        }
        sign * (hours * 3600 + minutes * 60)
    };
    let days = days_from_civil(year, month, day);
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))?
        .checked_sub(offset)?;
    (seconds >= 0).then_some(seconds as u64)
}

// Howard Hinnant's proleptic Gregorian conversion, offset to Unix epoch.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = i64::from(if month > 2 { month - 3 } else { month + 9 });
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

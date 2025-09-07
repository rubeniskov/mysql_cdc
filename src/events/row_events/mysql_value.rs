use std::convert::TryFrom;
use std::num::{TryFromIntError};
#[cfg(any(feature = "rust_decimal", feature = "bigdecimal"))]
use std::str::FromStr;

#[derive(Debug)]
pub enum ConvError {
    OutOfRange(&'static str),
    Parse(&'static str),
    Type(&'static str),
    Decimal(&'static str),
}

impl From<TryFromIntError> for ConvError {
    fn from(_: TryFromIntError) -> Self { ConvError::OutOfRange("integer conversion") }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug)]
pub struct Date { pub year: u16, pub month: u8, pub day: u8 }

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug)]
pub struct Time { // MySQL TIME; can be negative and up to ±838:59:59.999
    pub hour: i16,  // -838..= 838
    pub minute: u8, // 0..=59
    pub second: u8, // 0..=59
    pub millis: u32,// 0..=999
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug)]
pub struct DateTime {
    pub year: u16, pub month: u8, pub day: u8,
    pub hour: u8, pub minute: u8, pub second: u8, pub millis: u32,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug)]
pub enum MySqlValue {
    TinyInt(u8),
    SmallInt(u16),
    MediumInt(u32),
    Int(u32),
    BigInt(u64),
    Float(f32),
    Double(f64),
    Decimal(String),
    String(String),
    Bit(Vec<bool>),
    Enum(u32),
    Set(u64),
    Blob(Vec<u8>),
    Year(u16),
    Date(Date),
    Time(Time),
    DateTime(DateTime),
    Timestamp(u64), // millis since unix epoch (UTC)
}

// ---------- Helpers ----------
fn pack_bits_to_bytes(bits: &[bool]) -> Vec<u8> {
    let mut out = vec![0u8; (bits.len() + 7) / 8];
    for (i, b) in bits.iter().enumerate() {
        if *b {
            out[i / 8] |= 1u8 << (i % 8);
        }
    }
    out
}

// ---------- chrono conversions (enable with `features = ["chrono"]`) ----------
#[cfg(feature = "chrono")]
mod chrono_conv {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Duration, TimeZone, Utc};

    impl TryFrom<&Date> for NaiveDate {
        type Error = ConvError;
        fn try_from(d: &Date) -> Result<Self, Self::Error> {
            NaiveDate::from_ymd_opt(d.year as i32, d.month as u32, d.day as u32)
                .ok_or(ConvError::OutOfRange("invalid date"))
        }
    }

    // MySQL TIME -> chrono::Duration (supports negatives & >24h)
    impl TryFrom<&Time> for Duration {
        type Error = ConvError;
        fn try_from(t: &Time) -> Result<Self, Self::Error> {
            if t.minute > 59 || t.second > 59 || t.millis > 999 {
                return Err(ConvError::OutOfRange("invalid time components"));
            }
            let sign = if t.hour < 0 { -1 } else { 1 };
            let abs_h = t.hour.unsigned_abs() as i64;
            let total_ms: i64 = sign as i64
                * ((abs_h * 3600_000)
                + (t.minute as i64 * 60_000)
                + (t.second as i64 * 1_000)
                + (t.millis as i64));
            Ok(Duration::milliseconds(total_ms))
        }
    }

    impl TryFrom<&DateTime> for NaiveDateTime {
        type Error = ConvError;
        fn try_from(dt: &DateTime) -> Result<Self, Self::Error> {
            let date = chrono::NaiveDate::try_from(&Date { year: dt.year, month: dt.month, day: dt.day })?;
            let time = NaiveTime::from_hms_milli_opt(dt.hour as u32, dt.minute as u32, dt.second as u32, dt.millis)
                .ok_or(ConvError::OutOfRange("invalid datetime time part"))?;
            Ok(NaiveDateTime::new(date, time))
        }
    }

    // TIMESTAMP (millis) -> NaiveDateTime (UTC, no offset)
    impl TryFrom<&MySqlValue> for NaiveDateTime {
        type Error = ConvError;
        fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
            match v {
                MySqlValue::DateTime(dt) => NaiveDateTime::try_from(dt),
                MySqlValue::Timestamp(ms) => {
                    let secs = (*ms / 1000) as i64;
                    let sub_ms = (*ms % 1000) as u32;
                    let ndt = Utc
                        .timestamp_millis_opt((secs * 1000) + sub_ms as i64)
                        .single()
                        .ok_or(ConvError::OutOfRange("invalid timestamp millis"))?
                        .naive_utc();
                    Ok(ndt)
                }
                _ => Err(ConvError::Type("expected DateTime/Timestamp")),
            }
        }
    }

    impl TryFrom<&MySqlValue> for NaiveDate {
        type Error = ConvError;
        fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
            match v {
                MySqlValue::Date(d) => NaiveDate::try_from(d),
                _ => Err(ConvError::Type("expected Date")),
            }
        }
    }

    // Clock time from TIME, only when it fits 0..24h and non-negative
    impl TryFrom<&MySqlValue> for NaiveTime {
        type Error = ConvError;
        fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
            match v {
                MySqlValue::Time(t) => {
                    if t.hour < 0 || t.hour > 23 {
                        return Err(ConvError::OutOfRange("TIME outside 0..24h; use Duration"));
                    }
                    NaiveTime::from_hms_milli_opt(t.hour as u32, t.minute as u32, t.second as u32, t.millis)
                        .ok_or(ConvError::OutOfRange("invalid time"))
                }
                _ => Err(ConvError::Type("expected Time")),
            }
        }
    }
}

// ---------- time crate conversions (enable with `features = ["time"]`) ----------
#[cfg(feature = "time")]
mod time_conv {
    use super::*;
    use time::{Date as TDate, Time as TTime, PrimitiveDateTime, Duration, OffsetDateTime};

    impl TryFrom<&Date> for TDate {
        type Error = ConvError;
        fn try_from(d: &Date) -> Result<Self, Self::Error> {
            TDate::from_calendar_date(d.year as i32, 
                time::Month::try_from(d.month).map_err(|_| ConvError::OutOfRange("month"))?, 
                d.day).map_err(|_| ConvError::OutOfRange("date"))
        }
    }

    // MySQL TIME -> time::Duration (supports negatives & >24h)
    impl TryFrom<&Time> for Duration {
        type Error = ConvError;
        fn try_from(t: &Time) -> Result<Self, Self::Error> {
            if t.minute > 59 || t.second > 59 || t.millis > 999 {
                return Err(ConvError::OutOfRange("invalid time components"));
            }
            let sign = if t.hour < 0 { -1 } else { 1 };
            let abs_h = t.hour.unsigned_abs() as i64;
            let total_ms = (abs_h * 3_600_000) + (t.minute as i64 * 60_000) + (t.second as i64 * 1000) + (t.millis as i64);
            Ok(Duration::milliseconds(total_ms) * sign)
        }
    }

    impl TryFrom<&DateTime> for PrimitiveDateTime {
        type Error = ConvError;
        fn try_from(dt: &DateTime) -> Result<Self, Self::Error> {
            let d = TDate::try_from(&Date { year: dt.year, month: dt.month, day: dt.day })?;
            let t = TTime::from_hms_milli(dt.hour, dt.minute, dt.second, dt.millis as u16)
                .map_err(|_| ConvError::OutOfRange("invalid time"))?;
            Ok(PrimitiveDateTime::new(d, t))
        }
    }

    // TIMESTAMP (millis) -> OffsetDateTime (UTC)
    impl TryFrom<&MySqlValue> for OffsetDateTime {
        type Error = ConvError;
        fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
            match v {
                MySqlValue::Timestamp(ms) => {
                    let secs = (*ms / 1000) as i64;
                    let nanos = ((*ms % 1000) * 1_000_000) as i32;
                    Ok(OffsetDateTime::from_unix_timestamp(secs)
                        .map_err(|_| ConvError::OutOfRange("timestamp secs"))?
                        .replace_nanosecond(nanos as u32).unwrap())
                }
                _ => Err(ConvError::Type("expected Timestamp")),
            }
        }
    }
}

// ---------- decimal conversions ----------
#[cfg(feature = "bigdecimal")]
impl TryFrom<&MySqlValue> for bigdecimal::BigDecimal {
    type Error = ConvError;
    fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
        match v {
            MySqlValue::Decimal(s) => bigdecimal::BigDecimal::from_str(s)
                .map_err(|_| ConvError::Decimal("bigdecimal")),
            _ => Err(ConvError::Type("expected Decimal")),
        }
    }
}

#[cfg(feature = "rust_decimal")]
impl TryFrom<&MySqlValue> for rust_decimal::Decimal {
    type Error = ConvError;
    fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
        match v {
            MySqlValue::Decimal(s) => rust_decimal::Decimal::from_str(s)
                .map_err(|_| ConvError::Decimal("rust_decimal")),
            _ => Err(ConvError::Type("expected Decimal")),
        }
    }
}

/// Implement `TryFrom<&MySqlValue>` for an integer target by mapping a set of
/// integer-like variants to `T::try_from(*x)`.
macro_rules! impl_try_from_ints {
    ($t:ty, $( $variant:ident ),+ $(,)?) => {
        impl TryFrom<&MySqlValue> for $t {
            type Error = ConvError;
            fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
                match v {
                    $(
                        MySqlValue::$variant(x) => <$t>::try_from(*x)
                            .map_err(|_| ConvError::OutOfRange(stringify!($variant))),
                    )+
                    _ => Err(ConvError::Type(concat!(
                        "not an integer-like value (expected one of: ",
                        $( stringify!($variant), " ", )+
                        ")"
                    ))),
                }
            }
        }
    };
}

// Signed targets
impl_try_from_ints!(i8,  TinyInt, SmallInt, MediumInt, Int, BigInt, Enum, Set);
impl_try_from_ints!(i16, TinyInt, SmallInt, MediumInt, Int, BigInt, Enum, Set);
impl_try_from_ints!(i32, TinyInt, SmallInt, MediumInt, Int, BigInt, Enum, Set);
impl_try_from_ints!(i64, TinyInt, SmallInt, MediumInt, Int, BigInt, Enum, Set);

// Unsigned targets
impl_try_from_ints!(u8,  TinyInt, SmallInt, MediumInt, Int, BigInt, Enum, Set);
impl_try_from_ints!(u16, TinyInt, SmallInt, MediumInt, Int, BigInt, Enum, Set);
impl_try_from_ints!(u32, TinyInt, SmallInt, MediumInt, Int, BigInt, Enum, Set);
impl_try_from_ints!(u64, TinyInt, SmallInt, MediumInt, Int, BigInt, Enum, Set);

impl TryFrom<&MySqlValue> for f32 {
    type Error = ConvError;
    fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
        match v {
            MySqlValue::Float(f) => Ok(*f),
            MySqlValue::Double(d) => Ok(*d as f32),
            MySqlValue::String(s) => s.parse::<f32>().map_err(|_| ConvError::Parse("f32 from string")),
            _ => Err(ConvError::Type("expected Float/Double/String")),
        }
    }
}

impl TryFrom<&MySqlValue> for f64 {
    type Error = ConvError;
    fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
        match v {
            MySqlValue::Double(d) => Ok(*d),
            MySqlValue::Float(f)  => Ok(*f as f64),
            MySqlValue::String(s) => s.parse::<f64>().map_err(|_| ConvError::Parse("f64 from string")),
            _ => Err(ConvError::Type("expected Double/Float/String")),
        }
    }
}

impl TryFrom<&MySqlValue> for String {
    type Error = ConvError;
    fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
        match v {
            MySqlValue::String(s) => Ok(s.clone()),
            MySqlValue::Decimal(s) => Ok(s.clone()),
            MySqlValue::Year(y) => Ok(y.to_string()),
            _ => Err(ConvError::Type("expected textual value")),
        }
    }
}

impl TryFrom<&MySqlValue> for Vec<u8> {
    type Error = ConvError;
    fn try_from(v: &MySqlValue) -> Result<Self, Self::Error> {
        match v {
            MySqlValue::Blob(b) => Ok(b.clone()),
            MySqlValue::Bit(bits) => Ok(pack_bits_to_bytes(bits)),
            _ => Err(ConvError::Type("expected blob/bit")),
        }
    }
}


// ---------- convenience getters that mirror sqlx ergonomics ----------
impl MySqlValue {
    pub fn as_bool(&self) -> Result<bool, ConvError> {
        match self {
            MySqlValue::Bit(bits) if bits.len() == 1 => Ok(bits[0]),
            MySqlValue::TinyInt(x) => Ok(*x != 0),
            MySqlValue::SmallInt(x) => Ok(*x != 0),
            MySqlValue::Int(x) => Ok(*x != 0),
            MySqlValue::BigInt(x) => Ok(*x != 0),
            _ => Err(ConvError::Type("cannot view as bool")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------- integers --------

    #[test]
    fn int_conversions_success() {
        // TinyInt -> small targets
        assert_eq!(i8::try_from(&MySqlValue::TinyInt(42)).unwrap(), 42i8);
        assert_eq!(u8::try_from(&MySqlValue::TinyInt(200)).unwrap(), 200u8);

        // Int -> u32/i32
        assert_eq!(u32::try_from(&MySqlValue::Int(300)).unwrap(), 300u32);
        assert_eq!(i32::try_from(&MySqlValue::Int(123_456)).unwrap(), 123_456i32);

        // BigInt -> u64/i64
        let big = u64::MAX - 1;
        assert_eq!(u64::try_from(&MySqlValue::BigInt(big)).unwrap(), big);
        assert_eq!(i64::try_from(&MySqlValue::BigInt(9_223_372_036_854_775_807)).unwrap(), i64::MAX);

        // Enum/Set treated as integers
        assert_eq!(u32::try_from(&MySqlValue::Enum(5)).unwrap(), 5u32);
        assert_eq!(u64::try_from(&MySqlValue::Set(7)).unwrap(), 7u64);
    }

    #[test]
    fn int_conversions_out_of_range() {
        // Too big for i8
        let err = i8::try_from(&MySqlValue::BigInt(256)).unwrap_err();
        matches_out_of_range(err);

        // Negative range is not represented by our enum (unsigned carriers), but medium->i8 overflow:
        let err = i8::try_from(&MySqlValue::MediumInt(u32::from(u16::MAX) + 1)).unwrap_err();
        matches_out_of_range(err);

        let err = i32::try_from(&MySqlValue::BigInt((i32::MAX as u64) + 1)).unwrap_err();
        matches_out_of_range(err);
    }

    fn matches_out_of_range(err: ConvError) {
        match err {
            ConvError::OutOfRange(_) => {}
            other => panic!("expected OutOfRange, got {:?}", other),
        }
    }

    // -------- floats / strings / bytes / bits --------

    #[test]
    fn float_and_string_conversions() {
        assert_eq!(f32::try_from(&MySqlValue::Float(3.5)).unwrap(), 3.5f32);
        assert_eq!(f64::try_from(&MySqlValue::Double(2.25)).unwrap(), 2.25f64);
        assert_eq!(f64::try_from(&MySqlValue::String("3.14".into())).unwrap(), 3.14f64);

        assert_eq!(
            String::try_from(&MySqlValue::String("hello".into())).unwrap(),
            "hello".to_string()
        );
        assert_eq!(
            String::try_from(&MySqlValue::Decimal("123.45".into())).unwrap(),
            "123.45".to_string()
        );
        assert_eq!(
            String::try_from(&MySqlValue::Year(2024)).unwrap(),
            "2024".to_string()
        );
    }

    #[test]
    fn blob_and_bit_conversions() {
        // Blob passthrough
        let data = vec![1, 2, 3, 255];
        assert_eq!(Vec::<u8>::try_from(&MySqlValue::Blob(data.clone())).unwrap(), data);

        // Bits -> packed bytes (LSB-first packing)
        // true,false,true,false,true,false,true,false => 0b0101_0101 = 0x55
        let bits = vec![true, false, true, false, true, false, true, false];
        let packed = Vec::<u8>::try_from(&MySqlValue::Bit(bits)).unwrap();
        assert_eq!(packed, vec![0x55u8]);
    }

    #[test]
    fn as_bool_works() {
        assert_eq!(MySqlValue::TinyInt(1).as_bool().unwrap(), true);
        assert_eq!(MySqlValue::SmallInt(0).as_bool().unwrap(), false);
        assert_eq!(MySqlValue::Int(2).as_bool().unwrap(), true);
        assert_eq!(MySqlValue::BigInt(0).as_bool().unwrap(), false);
        assert_eq!(MySqlValue::Bit(vec![true]).as_bool().unwrap(), true);

        // Wrong shape should error
        let err = MySqlValue::Blob(vec![1]).as_bool().unwrap_err();
        matches_type(err);
    }

    fn matches_type(err: ConvError) {
        match err {
            ConvError::Type(_) => {}
            other => panic!("expected Type, got {:?}", other),
        }
    }

    // -------- chrono feature --------

    #[cfg(feature = "chrono")]
    mod chrono_tests {
        use super::*;
        use chrono::{Datelike, Timelike};
        use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Duration};

        #[test]
        fn date_datetime_timestamp_to_chrono() {
            // Date -> NaiveDate
            let d = Date { year: 2024, month: 2, day: 29 };
            let nd: NaiveDate = (&d).try_into().unwrap();
            assert_eq!(nd.year(), 2024);
            assert_eq!(nd.month(), 2);
            assert_eq!(nd.day(), 29);

            // DateTime -> NaiveDateTime
            let dt = DateTime { year: 2023, month: 12, day: 31, hour: 23, minute: 59, second: 58, millis: 999 };
            let ndt: NaiveDateTime = (&dt).try_into().unwrap();
            assert_eq!(ndt.date().year(), 2023);
            assert_eq!(ndt.time().hour(), 23);
            assert_eq!(ndt.time().minute(), 59);
            assert_eq!(ndt.time().second(), 58);
            assert_eq!(ndt.time().nanosecond(), 999_000_000);

            // Timestamp (ms) -> NaiveDateTime UTC
            let ts = MySqlValue::Timestamp(1_000); // 1970-01-01 00:00:01.000
            let ndt2: NaiveDateTime = (&ts).try_into().unwrap();
            assert_eq!(ndt2.and_utc().timestamp_millis(), 1_000);
        }

        #[test]
        fn time_to_duration_and_naivetime() {
            // Duration with negatives and >24h
            let t = Time { hour: -1, minute: 0, second: 0, millis: 0 };
            let dur: Duration = (&t).try_into().unwrap();
            assert_eq!(dur.num_seconds(), -3600);

            let t2 = Time { hour: 25, minute: 0, second: 0, millis: 0 };
            let err = NaiveTime::try_from(&MySqlValue::Time(t2)).unwrap_err();
            matches_out_of_range(err);

            // A valid clock time
            let t3 = Time { hour: 12, minute: 34, second: 56, millis: 789 };
            let nt: NaiveTime = (&MySqlValue::Time(t3)).try_into().unwrap();
            assert_eq!(nt.hour(), 12);
            assert_eq!(nt.minute(), 34);
            assert_eq!(nt.second(), 56);
            assert_eq!(nt.nanosecond(), 789_000_000);

            fn matches_out_of_range(err: ConvError) {
                match err {
                    ConvError::OutOfRange(_) => {}
                    other => panic!("expected OutOfRange, got {:?}", other),
                }
            }
        }
    }

    // -------- time feature --------

    #[cfg(feature = "time")]
    mod time_tests {
        use super::*;
        use time::{Date, PrimitiveDateTime, Duration, OffsetDateTime, Month};

        #[test]
        fn date_datetime_timestamp_to_time_crate() {
            // Date -> time::Date
            let d = super::Date { year: 2021, month: 3, day: 14 };
            let td: Date = (&d).try_into().unwrap();
            assert_eq!(td.year(), 2021);
            assert_eq!(td.month(), Month::March);
            assert_eq!(td.day(), 14);

            // DateTime -> PrimitiveDateTime
            let dt = super::DateTime { year: 2020, month: 1, day: 2, hour: 3, minute: 4, second: 5, millis: 6 };
            let pdt: PrimitiveDateTime = (&dt).try_into().unwrap();
            assert_eq!(pdt.date().year(), 2020);
            assert_eq!(pdt.time().hour(), 3);
            assert_eq!(pdt.time().millisecond(), 6);

            // Timestamp -> OffsetDateTime
            let ts = super::MySqlValue::Timestamp(2_000); // 1970-01-01T00:00:02Z
            let odt: OffsetDateTime = (&ts).try_into().unwrap();
            assert_eq!(odt.unix_timestamp(), 2);
        }

        #[test]
        fn time_to_duration_time_crate() {
            // Negative hour
            let t = super::Time { hour: -2, minute: 30, second: 0, millis: 0 };
            let dur: Duration = (&t).try_into().unwrap();
            assert_eq!(dur.whole_minutes(), -150);

            // Invalid components
            let bad = super::Time { hour: 0, minute: 61, second: 0, millis: 0 };
            let err = Duration::try_from(&bad).unwrap_err();
            match err {
                ConvError::OutOfRange(_) => {}
                other => panic!("expected OutOfRange, got {:?}", other),
            }
        }
    }

    // -------- decimal features --------

    #[cfg(feature = "bigdecimal")]
    #[test]
    fn bigdecimal_from_decimal_string() {
        let v = MySqlValue::Decimal("12345.6789".into());
        let dec: bigdecimal::BigDecimal = (&v).try_into().unwrap();
        assert_eq!(dec.to_string(), "12345.6789");
    }

    #[cfg(feature = "rust_decimal")]
    #[test]
    fn rust_decimal_from_decimal_string() {
        let v = MySqlValue::Decimal("42.500".into());
        let dec: rust_decimal::Decimal = (&v).try_into().unwrap();
        assert_eq!(dec.scale(), 3);
        assert_eq!(dec.mantissa(), 42500);
    }

    // -------- errors for wrong types --------

    #[test]
    fn wrong_type_errors() {
        // Trying to parse string as bytes yields Type error
        let err = Vec::<u8>::try_from(&MySqlValue::String("abc".into())).unwrap_err();
        matches_type(err);

        // Trying to parse blob as float yields Type error
        let err = f64::try_from(&MySqlValue::Blob(vec![1,2,3])).unwrap_err();
        matches_type(err);

        fn matches_type(err: ConvError) {
            match err {
                ConvError::Type(_) => {}
                other => panic!("expected Type, got {:?}", other),
            }
        }
    }
}

// ---------------- serde tests ----------------
#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::*;
    use serde_json::{self, from_str as json_from_str, to_string as json_to_string};

    // --- structs ---

    #[test]
    fn serde_roundtrip_date() {
        let d = Date { year: 2024, month: 2, day: 29 };
        let s = json_to_string(&d).unwrap();
        // e.g. {"year":2024,"month":2,"day":29}
        let back: Date = json_from_str(&s).unwrap();
        assert_eq!(back.year, 2024);
        assert_eq!(back.month, 2);
        assert_eq!(back.day, 29);
    }

    #[test]
    fn serde_roundtrip_time() {
        let t = Time { hour: -5, minute: 4, second: 3, millis: 2 };
        let s = json_to_string(&t).unwrap();
        let back: Time = json_from_str(&s).unwrap();
        assert_eq!(back.hour, -5);
        assert_eq!(back.minute, 4);
        assert_eq!(back.second, 3);
        assert_eq!(back.millis, 2);
    }

    #[test]
    fn serde_roundtrip_datetime() {
        let dt = DateTime { year: 2023, month: 12, day: 31, hour: 23, minute: 59, second: 58, millis: 999 };
        let s = json_to_string(&dt).unwrap();
        let back: DateTime = json_from_str(&s).unwrap();
        assert_eq!(back.year, 2023);
        assert_eq!(back.month, 12);
        assert_eq!(back.day, 31);
        assert_eq!(back.hour, 23);
        assert_eq!(back.minute, 59);
        assert_eq!(back.second, 58);
        assert_eq!(back.millis, 999);
    }

    // --- enum: simple tuple variants ---

    #[test]
    fn serde_enum_tuple_variants_simple() {
        // Tuple variants serialize as {"Variant": value}
        let j = json_to_string(&MySqlValue::TinyInt(7)).unwrap();
        assert_eq!(j, r#"{"TinyInt":7}"#);

        let j = json_to_string(&MySqlValue::Double(3.5)).unwrap();
        assert_eq!(j, r#"{"Double":3.5}"#);

        let j = json_to_string(&MySqlValue::String("hi".into())).unwrap();
        assert_eq!(j, r#"{"String":"hi"}"#);

        let j = json_to_string(&MySqlValue::Enum(5)).unwrap();
        assert_eq!(j, r#"{"Enum":5}"#);

        let j = json_to_string(&MySqlValue::Set(9)).unwrap();
        assert_eq!(j, r#"{"Set":9}"#);

        let j = json_to_string(&MySqlValue::Timestamp(1_234)).unwrap();
        assert_eq!(j, r#"{"Timestamp":1234}"#);
    }

    // --- enum: variants with struct payloads ---

    #[test]
    fn serde_enum_with_date_payload() {
        let v = MySqlValue::Date(Date { year: 2024, month: 1, day: 2 });
        let s = json_to_string(&v).unwrap();
        // Expect {"Date":{"year":2024,"month":1,"day":2}}
        // Do roundtrip and pattern match:
        let back: MySqlValue = json_from_str(&s).unwrap();
        match back {
            MySqlValue::Date(Date { year, month, day }) => {
                assert_eq!(year, 2024);
                assert_eq!(month, 1);
                assert_eq!(day, 2);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn serde_enum_with_time_payload() {
        let v = MySqlValue::Time(Time { hour: 12, minute: 34, second: 56, millis: 7 });
        let s = json_to_string(&v).unwrap();
        let back: MySqlValue = json_from_str(&s).unwrap();
        match back {
            MySqlValue::Time(Time { hour, minute, second, millis }) => {
                assert_eq!(hour, 12);
                assert_eq!(minute, 34);
                assert_eq!(second, 56);
                assert_eq!(millis, 7);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn serde_enum_with_datetime_payload() {
        let v = MySqlValue::DateTime(DateTime {
            year: 1999, month: 12, day: 31, hour: 23, minute: 59, second: 59, millis: 999
        });
        let s = json_to_string(&v).unwrap();
        let back: MySqlValue = json_from_str(&s).unwrap();
        match back {
            MySqlValue::DateTime(DateTime { year, month, day, hour, minute, second, millis }) => {
                assert_eq!(year, 1999);
                assert_eq!(month, 12);
                assert_eq!(day, 31);
                assert_eq!(hour, 23);
                assert_eq!(minute, 59);
                assert_eq!(second, 59);
                assert_eq!(millis, 999);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    // --- enum: vector payloads (Bit and Blob) ---

    #[test]
    fn serde_enum_bit_and_blob() {
        let v = MySqlValue::Bit(vec![true, false, true]);
        let s = json_to_string(&v).unwrap();
        let back: MySqlValue = json_from_str(&s).unwrap();
        match back {
            MySqlValue::Bit(bits) => assert_eq!(bits, vec![true, false, true]),
            other => panic!("unexpected: {:?}", other),
        }

        let v = MySqlValue::Blob(vec![0, 1, 255]);
        let s = json_to_string(&v).unwrap();
        let back: MySqlValue = json_from_str(&s).unwrap();
        match back {
            MySqlValue::Blob(bytes) => assert_eq!(bytes, vec![0, 1, 255]),
            other => panic!("unexpected: {:?}", other),
        }
    }

    // --- a mixed document: Vec<MySqlValue> roundtrip ---

    #[test]
    fn serde_roundtrip_vector_of_values() {
        let values = vec![
            MySqlValue::TinyInt(1),
            MySqlValue::String("x".into()),
            MySqlValue::Year(2025),
            MySqlValue::Date(Date { year: 2020, month: 5, day: 6 }),
            MySqlValue::Timestamp(123),
        ];
        let s = json_to_string(&values).unwrap();
        let back: Vec<MySqlValue> = json_from_str(&s).unwrap();
        assert_eq!(back.len(), 5);

        // spot-check a few variants:
        match &back[0] { MySqlValue::TinyInt(1) => {}, other => panic!("bad[0]: {:?}", other) }
        match &back[1] { MySqlValue::String(s) if s == "x" => {}, other => panic!("bad[1]: {:?}", other) }
        match &back[2] { MySqlValue::Year(2025) => {}, other => panic!("bad[2]: {:?}", other) }
        match &back[4] { MySqlValue::Timestamp(123) => {}, other => panic!("bad[4]: {:?}", other) }
    }
}

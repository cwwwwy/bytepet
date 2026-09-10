use bytepet_core::persona::Persona;
use time::{OffsetDateTime, UtcOffset};

pub fn local_now_text() -> String {
    let now = local_now();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute()
    )
}

pub fn fallback_greeting(persona: &Persona) -> String {
    if let Some(greeting) = persona
        .greeting
        .as_ref()
        .filter(|text| !text.trim().is_empty())
    {
        return greeting.clone();
    }
    match local_now().hour() {
        5..=10 => "早上好呀，今天也一起加油吧。".to_string(),
        11..=13 => "中午好，记得休息一下。".to_string(),
        14..=18 => "下午好，我在这儿陪着你。".to_string(),
        _ => "晚上好，今天过得怎么样？".to_string(),
    }
}

fn local_now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| {
        let utc = OffsetDateTime::now_utc();
        utc.to_offset(UtcOffset::UTC)
    })
}

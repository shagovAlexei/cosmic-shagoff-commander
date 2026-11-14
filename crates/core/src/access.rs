//! Cloud access: a 30-day trial, then a key. The key is one line `email sig global`: the
//! minisign signature (the release key, `just access-key`) of `MESSAGE` + email, trusted comment
//! the email. Checked offline.

/// Days the clouds work without a key, from the first cloud mount / add.
pub const TRIAL_DAYS: u64 = 30;
const MESSAGE: &str = "shagoff-commander access\n";
const DAY: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// A key for this e-mail.
    Key(String),
    /// Days of the trial left (≥ 1).
    Trial(u64),
    Over,
}

/// The e-mail of `key` when `pubkey` signed it.
fn signed_by(key: &str, pubkey: &str) -> Option<String> {
    let [email, sig, global] = key.split_whitespace().collect::<Vec<_>>()[..] else {
        return None;
    };
    let text = format!("untrusted comment: -\n{sig}\ntrusted comment: {email}\n{global}\n");
    let msg = format!("{MESSAGE}{email}");
    crate::update::signed(msg.as_bytes(), &text, pubkey)
        .ok()
        .map(|()| email.to_string())
}

/// `trial` = when it started (unix secs, 0 = not yet), `now` likewise.
pub fn status(key: &str, trial: u64, now: u64) -> Access {
    status_by(key, trial, now, crate::update::PUBLIC_KEY)
}

fn status_by(key: &str, trial: u64, now: u64, pubkey: &str) -> Access {
    if let Some(email) = signed_by(key, pubkey) {
        return Access::Key(email);
    }
    let used = if trial == 0 {
        0
    } else {
        now.saturating_sub(trial) / DAY
    };
    match TRIAL_DAYS.saturating_sub(used) {
        0 => Access::Over,
        n => Access::Trial(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test key (`minisign -G -W`) and its `just access-key a@b.c`.
    const PUB: &str = "RWRYW2CWhmXecx7sT6z4eomdfoc4NqTvDGuPFN/yyVSmfzeXpyUoa+U0";
    const KEY: &str = "a@b.c RURYW2CWhmXec1YDAWpMIklHhjl7LHq9jUU/m+BSrSZFrLQrekcX8jDQH3tpWCP+WtgAD1gs759S+gSMFtuWOMA4lfX5T2I3AQI= rmZ9V3dYsdbYUagWY76ZBPPohnK19o3NaVHdLTKu691YT5ISSCZ0eECPh1qS0XtrBP/2A/+fHRO981WwvWWKDQ==";

    #[test]
    fn a_signed_key_names_its_email_and_nothing_else_passes() {
        assert_eq!(signed_by(KEY, PUB).as_deref(), Some("a@b.c"));
        assert_eq!(
            signed_by(&format!("  {KEY}\n"), PUB).as_deref(),
            Some("a@b.c")
        );
        assert_eq!(signed_by(&KEY.replacen("a@b.c", "x@b.c", 1), PUB), None);
        assert_eq!(signed_by(KEY, crate::update::PUBLIC_KEY), None);
        assert_eq!(signed_by("", PUB), None);
        assert_eq!(signed_by("a@b.c junk junk", PUB), None);
        // As `just access-key` prints it.
        let made = "x@y.z RURYW2CWhmXec6gZXVsUuLJRiT11DkDhlOTRNCB9NZsjVrh6dKnW0H5uq6YGenNA+u+leJyh8NFNhw33AWDsd5aoOB5Gfwh57g4= W30DmoDR7XPi3BQ8KnJOn9xMxKv8JBii2kcLNvrmGG6XEBuKsIDwsC0y6O0uoECn1Hevdov7I/j5pbMc7PRSDA==";
        assert_eq!(signed_by(made, PUB).as_deref(), Some("x@y.z"));
    }

    #[test]
    fn the_trial_runs_a_month_from_its_start() {
        let t = 1_000_000_000;
        assert_eq!(status_by("", 0, t, PUB), Access::Trial(TRIAL_DAYS));
        assert_eq!(status_by("", t, t, PUB), Access::Trial(30));
        assert_eq!(status_by("", t, t + 29 * DAY + 5, PUB), Access::Trial(1));
        assert_eq!(status_by("", t, t + 30 * DAY, PUB), Access::Over);
        assert_eq!(
            status_by(KEY, t, t + 99 * DAY, PUB),
            Access::Key("a@b.c".into())
        );
    }
}

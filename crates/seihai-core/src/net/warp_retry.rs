//! Retrying the login that follows a `PLO_SERVERWARP` (178) when the new
//! server turns it away because the account is still in use.
//!
//! A warp closes the old connection and logs in again at once. GServer-v2
//! keeps the old player until it notices the old socket is gone, and until
//! then it answers a named account's new login with S>C 16 (`PLO_DISCMESSAGE`)
//! `Account is already in use.` and closes (`TPlayerLogin.cpp`). Guests skip
//! that check. GServer-v2 also lets a new login take the account over once
//! the old player has sent nothing for more than 30 s, so a schedule that
//! reaches past 30 s gets in even when the old socket never closes.
//!
//! The real clients do not retry (5.00r7's `GraalEngine.dll`: S>C 16 only
//! stores the text for the reconnect dialog, documentation/v5v6-profiles.md).
//! Seihai keeps their first try, made at once with a fresh key, and only adds
//! the retries after that refusal. Each retry is the same fresh login.

use std::time::{Duration, Instant};

use crate::protocol::layouts::ServerWarp;

/// Waits before each retry. They add up to 31 s, past GServer-v2's 30 s
/// idle rule, so the last try lands after the old player can be taken over.
pub const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];

/// A refusal counts only this soon after the login it answers. GServer-v2
/// sends it while handling the login, before any level.
pub const REFUSAL_WINDOW: Duration = Duration::from_secs(10);

/// True when a `PLO_DISCMESSAGE` text says the account is still logged in:
/// GServer-v2's `Account is already in use.`, and the `double login` wording
/// seen from the lab. Case and surrounding text do not matter.
pub fn is_account_in_use(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("already in use") || t.contains("account in use") || t.contains("double login")
}

/// The warp being followed, and how many retries it has had.
#[derive(Debug, Clone)]
pub struct WarpRetry {
    warp: ServerWarp,
    retries: usize,
    login_at: Instant,
    due: Option<Instant>,
}

impl WarpRetry {
    /// A warp's first login was just made at `now`.
    pub fn new(warp: ServerWarp, now: Instant) -> Self {
        Self { warp, retries: 0, login_at: now, due: None }
    }

    /// The warp being followed.
    pub fn warp(&self) -> &ServerWarp {
        &self.warp
    }

    /// Retries made so far.
    pub fn retries(&self) -> usize {
        self.retries
    }

    /// True while a retry is waiting to be made.
    pub fn pending(&self) -> bool {
        self.due.is_some()
    }

    /// The connection from the last login closed at `now`, with the server's
    /// `PLO_DISCMESSAGE` text if it sent one. Returns the wait before the next
    /// try when this was an in-use refusal of that login and retries are
    /// left; `None` means the close stands as an error.
    pub fn on_closed(&mut self, now: Instant, disconnect: Option<&str>) -> Option<Duration> {
        if self.due.is_some() {
            return None;
        }
        if !disconnect.is_some_and(is_account_in_use) {
            return None;
        }
        if now.saturating_duration_since(self.login_at) > REFUSAL_WINDOW {
            return None;
        }
        let wait = *RETRY_DELAYS.get(self.retries)?;
        self.due = Some(now + wait);
        Some(wait)
    }

    /// The warp to log in to again, once its wait is over. Marks the retry
    /// as made at `now`.
    pub fn take_due(&mut self, now: Instant) -> Option<ServerWarp> {
        let due = self.due?;
        if now < due {
            return None;
        }
        self.due = None;
        self.retries += 1;
        self.login_at = now;
        Some(self.warp.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warp() -> ServerWarp {
        ServerWarp { name: "Lab Server".into(), host: "127.0.0.1".into(), port: "14802".into() }
    }

    #[test]
    fn gserver_v2_refusal_text_is_recognised() {
        assert!(is_account_in_use("Account is already in use."));
        assert!(is_account_in_use("double login"));
        assert!(is_account_in_use("ACCOUNT IN USE"));
        assert!(!is_account_in_use("Someone else has logged into your account."));
        assert!(!is_account_in_use("Your client version is not allowed on this server."));
        assert!(!is_account_in_use(""));
    }

    #[test]
    fn retries_follow_the_schedule_then_give_up() {
        let t0 = Instant::now();
        let mut r = WarpRetry::new(warp(), t0);
        let mut now = t0;
        for (i, &d) in RETRY_DELAYS.iter().enumerate() {
            now += Duration::from_millis(50);
            assert_eq!(r.on_closed(now, Some("Account is already in use.")), Some(d), "retry {i}");
            assert!(r.pending());
            assert_eq!(r.take_due(now + d - Duration::from_millis(1)), None);
            now += d;
            assert_eq!(r.take_due(now), Some(warp()));
            assert!(!r.pending());
            assert_eq!(r.retries(), i + 1);
        }
        now += Duration::from_millis(50);
        assert_eq!(r.on_closed(now, Some("Account is already in use.")), None);
        let total: Duration = RETRY_DELAYS.iter().sum();
        assert!(total > Duration::from_secs(30), "the schedule must pass GServer-v2's 30 s idle rule");
    }

    #[test]
    fn other_closes_are_not_retried() {
        let t0 = Instant::now();
        let mut r = WarpRetry::new(warp(), t0);
        assert_eq!(r.on_closed(t0, None), None);
        assert_eq!(r.on_closed(t0, Some("Your client version is not allowed on this server.")), None);
        assert!(!r.pending());
    }

    #[test]
    fn a_late_refusal_is_not_retried() {
        let t0 = Instant::now();
        let mut r = WarpRetry::new(warp(), t0);
        assert_eq!(r.on_closed(t0 + REFUSAL_WINDOW + Duration::from_millis(1), Some("Account is already in use.")), None);
        // The window starts again at each retry's login.
        let mut r = WarpRetry::new(warp(), t0);
        let d = r.on_closed(t0, Some("Account is already in use.")).unwrap();
        let t1 = t0 + d + Duration::from_secs(20);
        assert!(r.take_due(t1).is_some());
        assert!(r.on_closed(t1 + Duration::from_secs(1), Some("Account is already in use.")).is_some());
    }

    #[test]
    fn nothing_is_due_without_a_refusal() {
        let t0 = Instant::now();
        let mut r = WarpRetry::new(warp(), t0);
        assert_eq!(r.take_due(t0 + Duration::from_secs(60)), None);
        assert_eq!(r.retries(), 0);
    }
}

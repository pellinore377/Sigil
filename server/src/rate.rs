use crate::{
    enrollment::{bearer, now},
    error,
    prekeys::authorize,
    store_error, with_store, AppState,
};
use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::Response,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const MAX_DEVICES: usize = 4096;
const IDLE: Duration = Duration::from_secs(60);

struct Bucket {
    credit: u64,
    capacity: u64,
    cost: u64,
    updated: Instant,
}
impl Bucket {
    fn new(burst: u64, period_ms: u64, now: Instant) -> Self {
        let cost = period_ms * 1_000_000;
        Self {
            credit: burst * cost,
            capacity: burst * cost,
            cost,
            updated: now,
        }
    }
    fn take(&mut self, now: Instant) -> Result<(), Duration> {
        let elapsed = now
            .saturating_duration_since(self.updated)
            .as_nanos()
            .min(self.capacity as u128) as u64;
        self.updated = self.updated.max(now);
        self.credit = self.credit.saturating_add(elapsed).min(self.capacity);
        if self.credit < self.cost {
            return Err(Duration::from_nanos(self.cost - self.credit));
        }
        self.credit -= self.cost;
        Ok(())
    }
}
struct Device {
    all: Bucket,
    writes: Bucket,
    last: Instant,
}
pub(crate) struct Limiter {
    global: Bucket,
    enrollment: Bucket,
    devices: HashMap<String, Device>,
}
impl Limiter {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            global: Bucket::new(100, 20, now),
            enrollment: Bucket::new(5, 5000, now),
            devices: HashMap::new(),
        }
    }
    fn device(&mut self, id: String, write: bool, now: Instant) -> Result<(), Rejection> {
        if !self.devices.contains_key(&id) && self.devices.len() >= MAX_DEVICES {
            self.devices
                .retain(|_, device| now.saturating_duration_since(device.last) < IDLE);
            if self.devices.len() >= MAX_DEVICES {
                return Err(Rejection::Capacity);
            }
        }
        let device = self.devices.entry(id).or_insert_with(|| Device {
            all: Bucket::new(60, 500, now),
            writes: Bucket::new(20, 2000, now),
            last: now,
        });
        device.last = device.last.max(now);
        device.all.take(now).map_err(Rejection::Rate)?;
        if write {
            device.writes.take(now).map_err(Rejection::Rate)?;
        }
        Ok(())
    }
}
enum Rejection {
    Rate(Duration),
    Capacity,
}
fn reject(reason: Rejection) -> Response {
    let (mut response, seconds) = match reason {
        Rejection::Rate(wait) => (
            error(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Request rate exceeded; retry later",
            ),
            wait.as_secs() + u64::from(wait.subsec_nanos() != 0),
        ),
        Rejection::Capacity => (
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "limiter_capacity",
                "Rate limiter is full; retry later",
            ),
            60,
        ),
    };
    if let Ok(value) = HeaderValue::from_str(&seconds.max(1).to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

pub(crate) async fn limit(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let anonymous_group = request.uri().path().starts_with("/groups/v0/");
    if !request.uri().path().starts_with("/client/v0/") && !anonymous_group {
        return next.run(request).await;
    }
    if request.headers().contains_key(header::ORIGIN) {
        return error(
            StatusCode::FORBIDDEN,
            "origin_not_allowed",
            "Browser sessions are not enabled",
        );
    }
    let enrollment = matches!(
        request.uri().path(),
        "/client/v0/enroll" | "/client/v0/reauthorize"
    );
    let instant = Instant::now();
    let result = {
        let mut limiter = match state.rate.lock() {
            Ok(value) => value,
            Err(_) => return reject(Rejection::Capacity),
        };
        limiter.global.take(instant).and_then(|()| {
            if enrollment {
                limiter.enrollment.take(instant)
            } else {
                Ok(())
            }
        })
    };
    if let Err(wait) = result {
        return reject(Rejection::Rate(wait));
    }
    if enrollment || anonymous_group {
        return next.run(request).await;
    }
    let credential = match bearer(request.headers()) {
        Ok(value) => value,
        Err(value) => return store_error(value),
    };
    // Resolve the stable device ID before allocating a limiter entry. Random bad
    // tokens cannot grow the map, and credential rotation does not reset a budget.
    let device = match with_store(state.clone(), move |store| {
        authorize(&store.0, &credential, now()?)
    })
    .await
    {
        Ok(value) => value,
        Err(value) => return store_error(value),
    };
    let write = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS | Method::DELETE
    );
    let result = {
        let mut limiter = match state.rate.lock() {
            Ok(value) => value,
            Err(_) => return reject(Rejection::Capacity),
        };
        limiter.device(device, write, Instant::now())
    };
    if let Err(reason) = result {
        return reject(reason);
    }
    // Handlers still authorize within their storage transaction: revocation can
    // happen after this lookup and must not be bypassed by the limiter.
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_refill_is_fractional_bounded_and_monotonic() {
        let start = Instant::now();
        let mut bucket = Bucket::new(2, 1000, start);
        bucket.take(start).unwrap();
        bucket.take(start).unwrap();
        assert_eq!(
            bucket.take(start + Duration::from_millis(250)),
            Err(Duration::from_millis(750))
        );
        assert_eq!(bucket.take(start), Err(Duration::from_millis(750)));
        bucket.take(start + Duration::from_secs(1)).unwrap();
        let later = start + Duration::from_secs(100);
        bucket.take(later).unwrap();
        bucket.take(later).unwrap();
        assert!(bucket.take(later).is_err());
    }
    #[test]
    fn device_budgets_are_isolated_and_reads_survive_write_exhaustion() {
        let start = Instant::now();
        let mut limiter = Limiter::new(start);
        for _ in 0..20 {
            assert!(limiter.device("alice".into(), true, start).is_ok());
        }
        assert!(matches!(
            limiter.device("alice".into(), true, start),
            Err(Rejection::Rate(_))
        ));
        assert!(limiter.device("alice".into(), false, start).is_ok());
        assert!(limiter.device("bob".into(), true, start).is_ok());
        assert!(limiter
            .device("alice".into(), true, start + Duration::from_secs(2))
            .is_ok());
    }
    #[test]
    fn map_capacity_does_not_evict_active_budgets() {
        let start = Instant::now();
        let mut limiter = Limiter::new(start);
        for n in 0..MAX_DEVICES {
            assert!(limiter.device(n.to_string(), false, start).is_ok());
        }
        assert!(matches!(
            limiter.device("extra".into(), false, start),
            Err(Rejection::Capacity)
        ));
        assert_eq!(limiter.devices.len(), MAX_DEVICES);
        assert!(limiter
            .device("0".into(), false, start + Duration::from_secs(59))
            .is_ok());
        assert!(limiter.device("extra".into(), false, start + IDLE).is_ok());
        assert_eq!(limiter.devices.len(), 2);
        assert!(limiter.devices.contains_key("0"));
    }
    #[test]
    fn retry_after_rounds_up() {
        let response = reject(Rejection::Rate(Duration::from_millis(1100)));
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "2");
    }
}

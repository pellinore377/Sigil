use std::sync::Mutex;

#[derive(Default)]
pub(super) struct SendLanes([Mutex<()>; 3]);
impl SendLanes {
    pub(super) fn run<T>(&self, kind: usize, send: impl FnOnce() -> T) -> Option<T> {
        let _lane = self.0.get(kind)?.lock().ok()?;
        Some(send())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;
    #[test]
    fn preparation_and_transmission_are_serial_per_kind_only() {
        let lanes = Arc::new(SendLanes::default());
        let (events, seen) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let first = {
            let lanes = lanes.clone();
            let events = events.clone();
            std::thread::spawn(move || {
                lanes
                    .run(1, || {
                        events.send("video prepared").unwrap();
                        wait.recv().unwrap();
                        events.send("video written").unwrap();
                    })
                    .unwrap()
            })
        };
        assert_eq!(
            seen.recv_timeout(Duration::from_secs(2)).unwrap(),
            "video prepared"
        );
        assert!(lanes.0[1].try_lock().is_err());
        let (attempt, attempted) = mpsc::channel();
        let second = {
            let lanes = lanes.clone();
            let events = events.clone();
            std::thread::spawn(move || {
                attempt.send(()).unwrap();
                lanes
                    .run(1, || {
                        events.send("next video prepared").unwrap();
                        events.send("next video written").unwrap();
                    })
                    .unwrap()
            })
        };
        attempted.recv_timeout(Duration::from_secs(2)).unwrap();
        lanes
            .run(0, || events.send("audio written").unwrap())
            .unwrap();
        assert_eq!(
            seen.recv_timeout(Duration::from_secs(2)).unwrap(),
            "audio written"
        );
        assert!(seen.try_recv().is_err());
        release.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(
            seen.into_iter().take(3).collect::<Vec<_>>(),
            vec!["video written", "next video prepared", "next video written"]
        );
    }
}

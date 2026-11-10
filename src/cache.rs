use std::{future::Future, sync::Arc, time::Duration};

use tokio::{sync::Mutex, time::Instant};

/// A single cached value that expires after `ttl`. Concurrent callers on a miss wait for one
/// refresh instead of each fetching. Failed fetches are not cached.
#[derive(Debug)]
pub struct TtlCache<T> {
    ttl: Duration,
    slot: Mutex<Option<(Instant, Arc<T>)>>,
}

impl<T> TtlCache<T> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            slot: Mutex::new(None),
        }
    }

    /// The cached value if fresh, otherwise the result of `fetch` (cached on success).
    pub async fn get_or_try_refresh<F, Fut, E>(&self, fetch: F) -> Result<Arc<T>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let mut slot = self.slot.lock().await;
        if let Some((fetched_at, value)) = slot.as_ref()
            && fetched_at.elapsed() < self.ttl
        {
            return Ok(Arc::clone(value));
        }
        let value = Arc::new(fetch().await?);
        *slot = Some((Instant::now(), Arc::clone(&value)));
        Ok(value)
    }

    pub async fn invalidate(&self) {
        *self.slot.lock().await = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TTL: Duration = Duration::from_secs(60);

    async fn fetch_counted(calls: &AtomicUsize) -> Result<usize, String> {
        Ok(calls.fetch_add(1, Ordering::SeqCst) + 1)
    }

    #[tokio::test(start_paused = true)]
    async fn caches_until_ttl_expires() {
        let cache = TtlCache::new(TTL);
        let calls = AtomicUsize::new(0);

        assert_eq!(
            *cache
                .get_or_try_refresh(|| fetch_counted(&calls))
                .await
                .unwrap(),
            1
        );
        tokio::time::advance(TTL - Duration::from_secs(1)).await;
        assert_eq!(
            *cache
                .get_or_try_refresh(|| fetch_counted(&calls))
                .await
                .unwrap(),
            1
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(
            *cache
                .get_or_try_refresh(|| fetch_counted(&calls))
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn errors_are_not_cached_and_invalidate_forces_refetch() {
        let cache = TtlCache::new(TTL);
        let calls = AtomicUsize::new(0);

        let err = cache
            .get_or_try_refresh(|| async { Err::<usize, _>("canvas down".to_owned()) })
            .await;
        assert!(err.is_err());
        assert_eq!(
            *cache
                .get_or_try_refresh(|| fetch_counted(&calls))
                .await
                .unwrap(),
            1
        );

        cache.invalidate().await;
        assert_eq!(
            *cache
                .get_or_try_refresh(|| fetch_counted(&calls))
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_misses_fetch_once() {
        let cache = TtlCache::new(TTL);
        let calls = AtomicUsize::new(0);
        let slow_fetch = || async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            fetch_counted(&calls).await
        };

        let (a, b) = tokio::join!(
            cache.get_or_try_refresh(slow_fetch),
            cache.get_or_try_refresh(slow_fetch)
        );
        assert_eq!((*a.unwrap(), *b.unwrap()), (1, 1));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

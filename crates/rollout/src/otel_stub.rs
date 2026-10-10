// New in ether: stand-in for codex-otel's global metrics client with the same call shape.
// ether records no OpenTelemetry metrics, so `global()` is always `None` and `MetricsClient`
// has no values; every metrics call site in this crate becomes inert.

use std::time::Duration;

#[derive(Clone, Debug)]
pub(crate) enum MetricsClient {}

pub(crate) fn global() -> Option<MetricsClient> {
    None
}

impl MetricsClient {
    pub(crate) fn counter(&self, _name: &str, _inc: i64, _tags: &[(&str, &str)]) -> Result<(), ()> {
        match *self {}
    }

    pub(crate) fn histogram(
        &self,
        _name: &str,
        _value: i64,
        _tags: &[(&str, &str)],
    ) -> Result<(), ()> {
        match *self {}
    }

    pub(crate) fn histogram_with_boundaries<B>(
        &self,
        _name: &str,
        _value: i64,
        _boundaries: B,
        _tags: &[(&str, &str)],
    ) -> Result<(), ()> {
        match *self {}
    }

    pub(crate) fn record_duration(
        &self,
        _name: &str,
        _duration: Duration,
        _tags: &[(&str, &str)],
    ) -> Result<(), ()> {
        match *self {}
    }
}

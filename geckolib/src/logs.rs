#[cfg(feature = "log")]
#[macro_export]
macro_rules! log {
    (target: $target:expr, $lvl:expr, $($key:tt = $value:expr),+; $($arg:tt)+) => (log::log!(target: $target, $lvl, $($key = $value),+; $($arg)+));

    (target: $target:expr, $lvl:expr, $($arg:tt)+) => (log::log!(target: $target, $lvl, $($arg)+));

    ($lvl:expr, $($arg:tt)+) => (log::log!($lvl, $($arg)+));
}

#[cfg(feature = "log")]
#[macro_export]
macro_rules! error {
    (target: $target:expr, $($arg:tt)+) => (log::error!(target: $target, $($arg)+));

    ($($arg:tt)+) => (log::error!($($arg)+))
}

#[cfg(feature = "log")]
#[macro_export]
macro_rules! warn {
    (target: $target:expr, $($arg:tt)+) => (log::warn!(target: $target, $($arg)+));

    ($($arg:tt)+) => (log::warn!($($arg)+))
}

#[cfg(feature = "log")]
#[macro_export]
macro_rules! info {
    (target: $target:expr, $($arg:tt)+) => (log::info!(target: $targeto, $($arg)+));

    ($($arg:tt)+) => (log::info!($($arg)+))
}

#[cfg(feature = "log")]
#[macro_export]
macro_rules! debug {
    (target: $target:expr, $($arg:tt)+) => (log::debug!(target: $target, $($arg)+));

    ($($arg:tt)+) => (log::debug!($($arg)+))
}

#[cfg(feature = "log")]
#[macro_export]
macro_rules! trace {
    (target: $target:expr, $($arg:tt)+) => (log::trace!(target: $target, $($arg)+));

    ($($arg:tt)+) => (log::trace!($($arg)+))
}

#[cfg(not(feature = "log"))]
#[macro_export]
macro_rules! log {
    // log!(target: "my_target", Level::Info, key1 = 42, key2 = true; "a {} event", "log");
    (target: $target:expr, $lvl:expr, $($key:tt = $value:expr),+; $($arg:tt)+) => ();

    // log!(target: "my_target", Level::Info, "a {} event", "log");
    (target: $target:expr, $lvl:expr, $($arg:tt)+) => ();

    // log!(Level::Info, "a log event")
    ($lvl:expr, $($arg:tt)+) => ();
}

#[cfg(not(feature = "log"))]
#[macro_export]
macro_rules! error {
    (target: $target:expr, $($arg:tt)+) => ();

    ($($arg:tt)+) => ();
}

#[cfg(not(feature = "log"))]
#[macro_export]
macro_rules! warn {
    (target: $target:expr, $($arg:tt)+) => ();

    ($($arg:tt)+) => ();
}

#[cfg(not(feature = "log"))]
#[macro_export]
macro_rules! info {
    (target: $target:expr, $($arg:tt)+) => ();

    ($($arg:tt)+) => ();
}

#[cfg(not(feature = "log"))]
#[macro_export]
macro_rules! debug {
    (target: $target:expr, $($arg:tt)+) => ();

    ($($arg:tt)+) => ();
}

#[cfg(not(feature = "log"))]
#[macro_export]
macro_rules! trace {
    (target: $target:expr, $($arg:tt)+) => ();

    ($($arg:tt)+) => ();
}

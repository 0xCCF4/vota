//! Background jobs that run beside the server, such as the purge sweep. They
//! schedule work; the SQL they run lives in [`crate::data`].

pub mod purge_sweep;

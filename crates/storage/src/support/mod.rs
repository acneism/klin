//! Вспомогательный код application-services, нужный places: работа с SQLite (`sql-support`),
//! идентификаторы синхронизации (`sync-guid`), метки времени (`types`), прерывание запросов
//! (`interrupt-support`). Перенесено из mozilla/application-services, `components/support`,
//! коммит 96b6f69902d143d6257c176c6e49e8c21e6d314b (MPL 2.0).

#![allow(dead_code)]

pub mod error;
pub mod guid;
pub mod interrupt;
pub mod sql;
pub mod types;

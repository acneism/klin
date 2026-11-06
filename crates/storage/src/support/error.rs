//! Замена крейта `error-support` из application-services: тот же интерфейс, но без отправки
//! отчётов и телеметрии — ошибки только пишутся в лог (в Klin нет телеметрии по умолчанию).

pub use log::{debug, error, info, trace, warn};

/// Внутренняя ошибка, переведённая во внешнюю (`ApiResult`).
pub struct ErrorHandling<E> {
    pub err: E,
}

impl<E: std::fmt::Display> ErrorHandling<E> {
    pub fn convert(err: E) -> Self {
        Self { err }
    }

    /// В application-services отправляет отчёт об ошибке; здесь — запись в лог.
    pub fn report_error(self, type_name: &str) -> Self {
        log::error!("{type_name}: {}", self.err);
        self
    }

    pub fn log_warning(self) -> Self {
        log::warn!("{}", self.err);
        self
    }

    pub fn log_info(self) -> Self {
        log::info!("{}", self.err);
        self
    }
}

pub trait GetErrorHandling {
    type ExternalError;
    fn get_error_handling(&self) -> ErrorHandling<Self::ExternalError>;
}

/// Записать ошибку в лог и вернуть результат без изменений.
macro_rules! trace_error {
    ($result:expr) => {{
        let result = $result;
        if let Err(e) = &result {
            log::trace!("{}:{}: {e}", file!(), line!());
        }
        result
    }};
}
pub(crate) use trace_error;

/// В application-services — отчёт об ошибке; здесь — запись в лог.
macro_rules! report_error {
    ($type_name:expr, $($arg:tt)*) => {
        log::error!("{}: {}", $type_name, format!($($arg)*))
    };
}
pub(crate) use report_error;

/// Для тестов application-services настраивает логирование; здесь ничего не нужно.
pub fn init_for_tests() {}

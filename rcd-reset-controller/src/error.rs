use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Hal(String),
    Matter(String),
    Sensor(String),
    Actuator(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Hal(msg)      => write!(f, "HAL error: {msg}"),
            AppError::Matter(msg)   => write!(f, "Matter error: {msg}"),
            AppError::Sensor(msg)   => write!(f, "Sensor error: {msg}"),
            AppError::Actuator(msg) => write!(f, "Actuator error: {msg}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<esp_idf_hal::sys::EspError> for AppError {
    fn from(e: esp_idf_hal::sys::EspError) -> Self {
        AppError::Hal(e.to_string())
    }
}

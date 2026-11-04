use std::{error::Error, io};

pub type AppResult<T> = Result<T, Box<dyn Error>>;

pub fn input_error(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::other(message.into()))
}

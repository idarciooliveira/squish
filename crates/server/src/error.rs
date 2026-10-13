//! API errors: an HTTP status plus the JSON body `{"error": code, "message": text}` (ADR-0018).

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    /// A code from ADR-0017, or one of the HTTP-only codes from ADR-0018.
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError { status, code, message: message.into() }
    }

    pub fn busy() -> Self {
        ApiError::new(StatusCode::TOO_MANY_REQUESTS, "busy", "every compression slot is taken, retry shortly")
    }
}

impl From<squish_core::Error> for ApiError {
    fn from(e: squish_core::Error) -> Self {
        use squish_core::Error;
        let status = match e {
            Error::Unsupported(_) | Error::Animated(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::TooManyPixels { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Error::Decode(_) => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError::new(status, e.code(), e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.code, "message": self.message }).to_string();
        let mut response = (self.status, [(header::CONTENT_TYPE, "application/json")], body).into_response();
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_core::{Error, Format};

    #[test]
    fn core_errors_map_to_the_adr_statuses() {
        let cases = [
            (Error::Unsupported(None), StatusCode::UNSUPPORTED_MEDIA_TYPE),
            (Error::Animated(Format::Png), StatusCode::UNSUPPORTED_MEDIA_TYPE),
            (Error::TooManyPixels { width: 1, height: 1, limit: 0 }, StatusCode::PAYLOAD_TOO_LARGE),
            (Error::Decode(String::new()), StatusCode::UNPROCESSABLE_ENTITY),
            (Error::Encode(String::new()), StatusCode::INTERNAL_SERVER_ERROR),
        ];
        for (error, status) in cases {
            let code = error.code();
            let api = ApiError::from(error);
            assert_eq!((api.status, api.code), (status, code));
        }
    }

    #[test]
    fn busy_says_when_to_retry() {
        let response = ApiError::busy().into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "1");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    }
}

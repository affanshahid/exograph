use std::env;

use actix_web::http::header::Header;
use actix_web::HttpRequest;
use actix_web_httpauth::headers::authorization::Authorization;
use actix_web_httpauth::headers::authorization::Bearer;
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{decode, DecodingKey, TokenData, Validation};
use serde_json::Value;

pub enum JwtAuthenticationError {
    ExpiredToken,
    TamperedToken,
    Unknown,
}

pub struct JwtAuthenticator {
    secret: String, // Shared secret for HS algorithms, public key for RSA/ES
}

const JWT_SECRET_PARAM: &str = "PAYAS_JWT_SECRET";

impl JwtAuthenticator {
    pub fn new_from_env() -> Self {
        Self::new(env::var(JWT_SECRET_PARAM).ok().unwrap())
    }

    fn new(secret: String) -> Self {
        JwtAuthenticator { secret }
    }

    // TODO: Expand to work with extenral authentication providers such as auth0 (that require JWK support)
    fn validate_jwt(&self, token: &str) -> Result<TokenData<Value>, jsonwebtoken::errors::Error> {
        decode::<Value>(
            &token,
            &DecodingKey::from_secret(self.secret.as_ref()),
            &Validation::default(),
        )
    }

    /// Extract authentication form the "Authorization" header with a bearer token
    /// The claim is deserialized into an opaque json `Value`, which will be eventually mapped
    /// to the declared user context model
    pub fn extract_authentication(
        &self,
        req: HttpRequest,
    ) -> Result<Option<Value>, JwtAuthenticationError> {
        match Authorization::<Bearer>::parse(&req) {
            Ok(auth) => {
                let scheme = auth.into_scheme();
                let token = scheme.token().as_ref();
                self.validate_jwt(token)
                    .map(|v| Some(v.claims))
                    .map_err(|err| match &err.kind() {
                        ErrorKind::InvalidSignature => JwtAuthenticationError::TamperedToken,
                        ErrorKind::ExpiredSignature => JwtAuthenticationError::ExpiredToken,
                        _ => JwtAuthenticationError::Unknown,
                    })
            }
            Err(_) => {
                // Either the "Authorization" header was absent or the next token wasn't "Bearer"
                // It is not an error to have no authorization header, since that indicates an anonymous user
                // and there may be queries allowed for such users.
                Ok(None)
            }
        }
    }
}
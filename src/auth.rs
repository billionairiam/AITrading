use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, TokenData, Validation, decode, encode};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;
use totp_rs::{Algorithm, Secret, TOTP};
use urlencoding;
use uuid::Uuid;

// JWT secret, can only be set once.
static JWT_SECRET: OnceCell<Vec<u8>> = OnceCell::new();
// Admin mode flag, atomically updatable.
static ADMIN_MODE: AtomicBool = AtomicBool::new(false);

// OTP Issuer name, a constant.
pub const OTP_ISSUER: &str = "AITrading";

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Bcrypt hashing error: {0}")]
    Bcrypt(#[from] bcrypt::BcryptError),
    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("TOTP error: {0}")]
    Totp(#[from] totp_rs::TotpUrlError),
    #[error("JWT secret has not been set")]
    JwtSecretNotSet,
    #[error("Invalid token")]
    InvalidToken,
}

/// Sets the global JWT secret.
/// This function can only be called successfully once.
pub fn set_jwt_secret(secret: &str) {
    let _ = JWT_SECRET.set(secret.as_bytes().to_vec());
}

/// Sets the global admin mode.
pub fn set_admin_mode(enabled: bool) {
    ADMIN_MODE.store(enabled, Ordering::Relaxed);
}

/// Checks if admin mode is currently enabled.
pub fn is_admin_mode() -> bool {
    ADMIN_MODE.load(Ordering::Relaxed)
}

// --- JWT Claims ---

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub user_id: String,
    pub email: String,
    // Registered claims
    exp: i64,    // Expiration time (as UTC timestamp)
    iat: i64,    // Issued at (as UTC timestamp)
    nbf: i64,    // Not before (as UTC timestamp)
    iss: String, // Issuer
}

// --- Core Authentication Functions ---

/// Hashes a password using bcrypt with the default cost.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)?;
    Ok(hash)
}

/// Verifies a password against a bcrypt hash.
pub fn check_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

/// Generates a new Base32-encoded secret key for TOTP.
///
/// The Go implementation had a slight redundancy where it generated random bytes
/// and then immediately discarded them. This version directly uses the TOTP library's
/// recommended way to generate a secret.
pub fn generate_otp_secret() -> Result<String, AuthError> {
    // We use the same parameters common authenticators expect (SHA1, 6 digits, 30s period)
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::generate_secret().to_bytes().unwrap(),
        Some(OTP_ISSUER.to_string()),
        Uuid::new_v4().to_string(),
    )?;
    Ok(totp.get_secret_base32())
}

/// Verifies a TOTP code against a secret.
pub fn verify_otp(secret: &str, code: &str) -> bool {
    let secret_bytes = Secret::Encoded(secret.to_string()).to_bytes().unwrap();

    let totp_result = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        Some(OTP_ISSUER.to_string()),
        "validation".to_string(), // Placeholder account name
    );

    match totp_result {
        Ok(totp) => totp.check_current(code).unwrap_or(false),
        Err(_) => false,
    }
}

/// Generates a new JWT for a given user.
pub fn generate_jwt(user_id: &str, email: &str) -> Result<String, AuthError> {
    let now = Utc::now();
    let expiration = now + Duration::hours(24);

    let claims = Claims {
        user_id: user_id.to_string(),
        email: email.to_string(),
        iat: now.timestamp(),
        nbf: now.timestamp(),
        exp: expiration.timestamp(),
        iss: "AITrading".to_string(),
    };

    let secret = JWT_SECRET.get().ok_or(AuthError::JwtSecretNotSet)?;
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret),
    )?;

    Ok(token)
}

/// Validates a JWT and returns the claims if successful.
pub fn validate_jwt(token_str: &str) -> Result<TokenData<Claims>, AuthError> {
    let secret = JWT_SECRET.get().ok_or(AuthError::JwtSecretNotSet)?;
    let token_data = decode::<Claims>(
        token_str,
        &DecodingKey::from_secret(secret),
        &Validation::new(jsonwebtoken::Algorithm::HS256),
    )?;
    Ok(token_data)
}

/// Generates a URL for a QR code that can be used to set up OTP.
pub fn get_otp_qrcode_url(secret: &str, email: &str) -> String {
    // The account name in the URL should be user-identifiable, like their email.
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}",
        urlencoding::encode(OTP_ISSUER),
        urlencoding::encode(email),
        secret,
        urlencoding::encode(OTP_ISSUER)
    )
}

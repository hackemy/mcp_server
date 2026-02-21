use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid or expired token")]
    InvalidToken,
    #[error("missing userId claim")]
    MissingClaim,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    #[serde(rename = "userId")]
    user_id: String,
    iat: u64,
    exp: u64,
}

/// Parse a JWT signed with HMAC-SHA256 and return the userId claim.
pub fn parse_token(secret: &str, token_str: &str) -> Result<String, AuthError> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();
    validation.set_required_spec_claims(&["exp"]);

    let data = decode::<Claims>(token_str, &key, &validation)
        .map_err(|_| AuthError::InvalidToken)?;

    if data.claims.user_id.is_empty() {
        return Err(AuthError::MissingClaim);
    }

    Ok(data.claims.user_id)
}

/// Create a JWT with the given userId claim, signed with HMAC-SHA256.
pub fn create_token(secret: &str, user_id: &str, expiry_secs: u64) -> Result<String, AuthError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = Claims {
        user_id: user_id.to_string(),
        iat: now,
        exp: now + expiry_secs,
    };

    let key = EncodingKey::from_secret(secret.as_bytes());
    encode(&Header::new(Algorithm::HS256), &claims, &key)
        .map_err(|_| AuthError::InvalidToken)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SECRET: &str = "test-secret-key-for-hmac256";

    #[test]
    fn test_create_and_parse_token() {
        let token = create_token(TEST_SECRET, "user-123", 3600).unwrap();
        let user_id = parse_token(TEST_SECRET, &token).unwrap();
        assert_eq!(user_id, "user-123");
    }

    #[test]
    fn test_parse_wrong_secret() {
        let token = create_token(TEST_SECRET, "user-123", 3600).unwrap();
        let result = parse_token("wrong-secret", &token);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_expired() {
        // Manually create a token with exp in the past.
        let past = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 3600; // 1 hour ago

        let claims = Claims {
            user_id: "user-123".to_string(),
            iat: past - 3600,
            exp: past,
        };

        let key = EncodingKey::from_secret(TEST_SECRET.as_bytes());
        let token = encode(&Header::new(Algorithm::HS256), &claims, &key).unwrap();
        let result = parse_token(TEST_SECRET, &token);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_string() {
        let result = parse_token(TEST_SECRET, "not-a-valid-jwt");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_and_parse_roundtrip() {
        let token = create_token(TEST_SECRET, "user-456", 3600).unwrap();
        let user_id = parse_token(TEST_SECRET, &token).unwrap();
        assert_eq!(user_id, "user-456");
    }
}

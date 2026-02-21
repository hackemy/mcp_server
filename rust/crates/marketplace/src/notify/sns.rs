use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum SnsError {
    #[error("sns publish to {0}: {1}")]
    Publish(String, String),
}

/// Trait abstracting SNS operations for testing.
#[async_trait]
pub trait SnsApi: Send + Sync {
    async fn send_sms(&self, phone: &str, message: &str) -> Result<(), SnsError>;
}

/// Real AWS SNS client.
pub struct SnsClient {
    client: aws_sdk_sns::Client,
}

impl SnsClient {
    pub fn new(config: &aws_config::SdkConfig) -> Self {
        Self {
            client: aws_sdk_sns::Client::new(config),
        }
    }
}

#[async_trait]
impl SnsApi for SnsClient {
    async fn send_sms(&self, phone: &str, message: &str) -> Result<(), SnsError> {
        self.client
            .publish()
            .phone_number(phone)
            .message(message)
            .send()
            .await
            .map_err(|e| SnsError::Publish(phone.into(), e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    pub struct MockSns {
        pub messages: Mutex<Vec<(String, String)>>,
    }

    impl MockSns {
        pub fn new() -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SnsApi for MockSns {
        async fn send_sms(&self, phone: &str, message: &str) -> Result<(), SnsError> {
            self.messages
                .lock()
                .unwrap()
                .push((phone.into(), message.into()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockSns;
    use super::*;

    #[tokio::test]
    async fn test_send_sms() {
        let sns = MockSns::new();
        sns.send_sms("+15551234567", "Your code is 123456")
            .await
            .unwrap();
        let msgs = sns.messages.lock().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, "+15551234567");
        assert_eq!(msgs[0].1, "Your code is 123456");
    }
}

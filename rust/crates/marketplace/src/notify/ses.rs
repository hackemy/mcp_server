use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum SesError {
    #[error("ses send to {0}: {1}")]
    Send(String, String),
}

/// Trait abstracting SES operations for testing.
#[async_trait]
pub trait SesApi: Send + Sync {
    async fn send_email(
        &self,
        from_addr: &str,
        to_addr: &str,
        subject: &str,
        body: &str,
    ) -> Result<(), SesError>;
}

/// Real AWS SES v2 client.
pub struct SesClient {
    client: aws_sdk_sesv2::Client,
}

impl SesClient {
    pub fn new(config: &aws_config::SdkConfig) -> Self {
        Self {
            client: aws_sdk_sesv2::Client::new(config),
        }
    }
}

#[async_trait]
impl SesApi for SesClient {
    async fn send_email(
        &self,
        from_addr: &str,
        to_addr: &str,
        subject: &str,
        body: &str,
    ) -> Result<(), SesError> {
        use aws_sdk_sesv2::types::{Body, Content, Destination, EmailContent, Message};

        let dest = Destination::builder().to_addresses(to_addr).build();
        let subject_content = Content::builder().data(subject).build().unwrap();
        let body_content = Content::builder().data(body).build().unwrap();
        let msg = Message::builder()
            .subject(subject_content)
            .body(Body::builder().text(body_content).build())
            .build();
        let content = EmailContent::builder().simple(msg).build();

        self.client
            .send_email()
            .from_email_address(from_addr)
            .destination(dest)
            .content(content)
            .send()
            .await
            .map_err(|e| SesError::Send(to_addr.into(), e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    pub struct MockSes {
        pub emails: Mutex<Vec<(String, String)>>,
    }

    impl MockSes {
        pub fn new() -> Self {
            Self {
                emails: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SesApi for MockSes {
        async fn send_email(
            &self,
            _from_addr: &str,
            to_addr: &str,
            subject: &str,
            _body: &str,
        ) -> Result<(), SesError> {
            self.emails
                .lock()
                .unwrap()
                .push((to_addr.into(), subject.into()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockSes;
    use super::*;

    #[tokio::test]
    async fn test_send_email() {
        let ses = MockSes::new();
        ses.send_email("noreply@example.com", "user@example.com", "Your OTP", "Code: 654321")
            .await
            .unwrap();
        let emails = ses.emails.lock().unwrap();
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].0, "user@example.com");
        assert_eq!(emails[0].1, "Your OTP");
    }
}

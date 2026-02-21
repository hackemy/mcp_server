use async_trait::async_trait;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client as DynamoDbClient;
use serde_json::Value;
use std::collections::HashMap;

/// Key pair for batch operations.
#[derive(Debug, Clone)]
pub struct KeyPair {
    pub pk: String,
    pub sk: String,
}

/// Trait abstracting DynamoDB operations for testing.
#[async_trait]
pub trait DynamoApi: Send + Sync {
    async fn put_item(
        &self,
        pk: &str,
        sk: &str,
        gsi1pk: &str,
        gsi1sk: &str,
        gsi2pk: &str,
        gsi2sk: &str,
        attrs: HashMap<String, Value>,
    ) -> Result<(), DynamoError>;

    async fn get_item(&self, pk: &str, sk: &str) -> Result<Option<HashMap<String, Value>>, DynamoError>;

    async fn query(&self, pk: &str) -> Result<Vec<HashMap<String, Value>>, DynamoError>;

    async fn query_with_sk(&self, pk: &str, sk: &str) -> Result<Vec<HashMap<String, Value>>, DynamoError>;

    async fn query_gsi(&self, index_name: &str, gsi_pk: &str) -> Result<Vec<HashMap<String, Value>>, DynamoError>;

    async fn query_gsi_with_sk(
        &self,
        index_name: &str,
        gsi_pk: &str,
        gsi_sk: &str,
    ) -> Result<Vec<HashMap<String, Value>>, DynamoError>;

    async fn delete_item(&self, pk: &str, sk: &str) -> Result<(), DynamoError>;

    async fn batch_delete_items(&self, items: &[KeyPair]) -> Result<(), DynamoError>;
}

#[derive(Debug, thiserror::Error)]
pub enum DynamoError {
    #[error("dynamo error: {0}")]
    Sdk(String),
    #[error("marshal error: {0}")]
    Marshal(String),
}

/// Real DynamoDB client implementation.
pub struct DynamoClient {
    client: DynamoDbClient,
    table_name: String,
}

impl DynamoClient {
    pub async fn new(table_name: &str) -> Result<Self, DynamoError> {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = DynamoDbClient::new(&config);
        Ok(Self {
            client,
            table_name: table_name.to_string(),
        })
    }
}

fn json_to_av(val: &Value) -> AttributeValue {
    match val {
        Value::String(s) => AttributeValue::S(s.clone()),
        Value::Number(n) => AttributeValue::N(n.to_string()),
        Value::Bool(b) => AttributeValue::Bool(*b),
        Value::Null => AttributeValue::Null(true),
        Value::Array(arr) => {
            let items: Vec<AttributeValue> = arr.iter().map(json_to_av).collect();
            AttributeValue::L(items)
        }
        Value::Object(map) => {
            let items: HashMap<String, AttributeValue> = map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_av(v)))
                .collect();
            AttributeValue::M(items)
        }
    }
}

fn av_to_json(av: &AttributeValue) -> Value {
    match av {
        AttributeValue::S(s) => Value::String(s.clone()),
        AttributeValue::N(n) => {
            if let Ok(i) = n.parse::<i64>() {
                Value::Number(i.into())
            } else if let Ok(f) = n.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(Value::Number)
                    .unwrap_or(Value::String(n.clone()))
            } else {
                Value::String(n.clone())
            }
        }
        AttributeValue::Bool(b) => Value::Bool(*b),
        AttributeValue::Null(_) => Value::Null,
        AttributeValue::L(items) => {
            Value::Array(items.iter().map(av_to_json).collect())
        }
        AttributeValue::M(map) => {
            let obj: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), av_to_json(v)))
                .collect();
            Value::Object(obj)
        }
        _ => Value::Null,
    }
}

fn items_to_json(items: &[HashMap<String, AttributeValue>]) -> Vec<HashMap<String, Value>> {
    items
        .iter()
        .map(|item| {
            item.iter()
                .map(|(k, v)| (k.clone(), av_to_json(v)))
                .collect()
        })
        .collect()
}

#[async_trait]
impl DynamoApi for DynamoClient {
    async fn put_item(
        &self,
        pk: &str,
        sk: &str,
        gsi1pk: &str,
        gsi1sk: &str,
        gsi2pk: &str,
        gsi2sk: &str,
        attrs: HashMap<String, Value>,
    ) -> Result<(), DynamoError> {
        let mut item: HashMap<String, AttributeValue> = HashMap::new();
        item.insert("PK".into(), AttributeValue::S(pk.into()));
        item.insert("SK".into(), AttributeValue::S(sk.into()));

        if !gsi1pk.is_empty() {
            item.insert("GSI1PK".into(), AttributeValue::S(gsi1pk.into()));
        }
        if !gsi1sk.is_empty() {
            item.insert("GSI1SK".into(), AttributeValue::S(gsi1sk.into()));
        }
        if !gsi2pk.is_empty() {
            item.insert("GSI2PK".into(), AttributeValue::S(gsi2pk.into()));
        }
        if !gsi2sk.is_empty() {
            item.insert("GSI2SK".into(), AttributeValue::S(gsi2sk.into()));
        }

        for (k, v) in &attrs {
            item.insert(k.clone(), json_to_av(v));
        }

        self.client
            .put_item()
            .table_name(&self.table_name)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| DynamoError::Sdk(e.to_string()))?;

        Ok(())
    }

    async fn get_item(&self, pk: &str, sk: &str) -> Result<Option<HashMap<String, Value>>, DynamoError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("PK", AttributeValue::S(pk.into()))
            .key("SK", AttributeValue::S(sk.into()))
            .send()
            .await
            .map_err(|e| DynamoError::Sdk(e.to_string()))?;

        Ok(out.item.map(|item| {
            item.iter()
                .map(|(k, v)| (k.clone(), av_to_json(v)))
                .collect()
        }))
    }

    async fn query(&self, pk: &str) -> Result<Vec<HashMap<String, Value>>, DynamoError> {
        let out = self
            .client
            .query()
            .table_name(&self.table_name)
            .key_condition_expression("PK = :pk")
            .expression_attribute_values(":pk", AttributeValue::S(pk.into()))
            .send()
            .await
            .map_err(|e| DynamoError::Sdk(e.to_string()))?;

        Ok(items_to_json(out.items()))
    }

    async fn query_with_sk(&self, pk: &str, sk: &str) -> Result<Vec<HashMap<String, Value>>, DynamoError> {
        let out = self
            .client
            .query()
            .table_name(&self.table_name)
            .key_condition_expression("PK = :pk AND SK = :sk")
            .expression_attribute_values(":pk", AttributeValue::S(pk.into()))
            .expression_attribute_values(":sk", AttributeValue::S(sk.into()))
            .send()
            .await
            .map_err(|e| DynamoError::Sdk(e.to_string()))?;

        Ok(items_to_json(out.items()))
    }

    async fn query_gsi(&self, index_name: &str, gsi_pk: &str) -> Result<Vec<HashMap<String, Value>>, DynamoError> {
        let pk_attr = if index_name == "GSI2" { "GSI2PK" } else { "GSI1PK" };

        let out = self
            .client
            .query()
            .table_name(&self.table_name)
            .index_name(index_name)
            .key_condition_expression(format!("{} = :pk", pk_attr))
            .expression_attribute_values(":pk", AttributeValue::S(gsi_pk.into()))
            .send()
            .await
            .map_err(|e| DynamoError::Sdk(e.to_string()))?;

        Ok(items_to_json(out.items()))
    }

    async fn query_gsi_with_sk(
        &self,
        index_name: &str,
        gsi_pk: &str,
        gsi_sk: &str,
    ) -> Result<Vec<HashMap<String, Value>>, DynamoError> {
        let (pk_attr, sk_attr) = if index_name == "GSI2" {
            ("GSI2PK", "GSI2SK")
        } else {
            ("GSI1PK", "GSI1SK")
        };

        let out = self
            .client
            .query()
            .table_name(&self.table_name)
            .index_name(index_name)
            .key_condition_expression(format!("{} = :pk AND {} = :sk", pk_attr, sk_attr))
            .expression_attribute_values(":pk", AttributeValue::S(gsi_pk.into()))
            .expression_attribute_values(":sk", AttributeValue::S(gsi_sk.into()))
            .send()
            .await
            .map_err(|e| DynamoError::Sdk(e.to_string()))?;

        Ok(items_to_json(out.items()))
    }

    async fn delete_item(&self, pk: &str, sk: &str) -> Result<(), DynamoError> {
        self.client
            .delete_item()
            .table_name(&self.table_name)
            .key("PK", AttributeValue::S(pk.into()))
            .key("SK", AttributeValue::S(sk.into()))
            .send()
            .await
            .map_err(|e| DynamoError::Sdk(e.to_string()))?;

        Ok(())
    }

    async fn batch_delete_items(&self, items: &[KeyPair]) -> Result<(), DynamoError> {
        if items.is_empty() {
            return Ok(());
        }

        for chunk in items.chunks(25) {
            let requests: Vec<aws_sdk_dynamodb::types::WriteRequest> = chunk
                .iter()
                .map(|kp| {
                    aws_sdk_dynamodb::types::WriteRequest::builder()
                        .delete_request(
                            aws_sdk_dynamodb::types::DeleteRequest::builder()
                                .key("PK", AttributeValue::S(kp.pk.clone()))
                                .key("SK", AttributeValue::S(kp.sk.clone()))
                                .build()
                                .unwrap(),
                        )
                        .build()
                })
                .collect();

            self.client
                .batch_write_item()
                .request_items(&self.table_name, requests)
                .send()
                .await
                .map_err(|e| DynamoError::Sdk(e.to_string()))?;
        }

        Ok(())
    }
}

// ── In-memory mock for testing ──

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    /// In-memory DynamoDB mock for testing.
    pub struct MockDynamo {
        items: Mutex<HashMap<String, HashMap<String, AttributeValue>>>,
    }

    impl MockDynamo {
        pub fn new() -> Self {
            Self {
                items: Mutex::new(HashMap::new()),
            }
        }

        fn make_key(pk: &str, sk: &str) -> String {
            format!("{}|{}", pk, sk)
        }
    }

    #[async_trait]
    impl DynamoApi for MockDynamo {
        async fn put_item(
            &self,
            pk: &str,
            sk: &str,
            gsi1pk: &str,
            gsi1sk: &str,
            gsi2pk: &str,
            gsi2sk: &str,
            attrs: HashMap<String, Value>,
        ) -> Result<(), DynamoError> {
            let mut item: HashMap<String, AttributeValue> = HashMap::new();
            item.insert("PK".into(), AttributeValue::S(pk.into()));
            item.insert("SK".into(), AttributeValue::S(sk.into()));
            if !gsi1pk.is_empty() {
                item.insert("GSI1PK".into(), AttributeValue::S(gsi1pk.into()));
            }
            if !gsi1sk.is_empty() {
                item.insert("GSI1SK".into(), AttributeValue::S(gsi1sk.into()));
            }
            if !gsi2pk.is_empty() {
                item.insert("GSI2PK".into(), AttributeValue::S(gsi2pk.into()));
            }
            if !gsi2sk.is_empty() {
                item.insert("GSI2SK".into(), AttributeValue::S(gsi2sk.into()));
            }
            for (k, v) in &attrs {
                item.insert(k.clone(), json_to_av(v));
            }
            let key = Self::make_key(pk, sk);
            self.items.lock().unwrap().insert(key, item);
            Ok(())
        }

        async fn get_item(&self, pk: &str, sk: &str) -> Result<Option<HashMap<String, Value>>, DynamoError> {
            let key = Self::make_key(pk, sk);
            let items = self.items.lock().unwrap();
            Ok(items.get(&key).map(|item| {
                item.iter().map(|(k, v)| (k.clone(), av_to_json(v))).collect()
            }))
        }

        async fn query(&self, pk: &str) -> Result<Vec<HashMap<String, Value>>, DynamoError> {
            let items = self.items.lock().unwrap();
            let results: Vec<_> = items
                .values()
                .filter(|item| {
                    item.get("PK")
                        .and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None })
                        == Some(pk)
                })
                .map(|item| item.iter().map(|(k, v)| (k.clone(), av_to_json(v))).collect())
                .collect();
            Ok(results)
        }

        async fn query_with_sk(&self, pk: &str, sk: &str) -> Result<Vec<HashMap<String, Value>>, DynamoError> {
            let key = Self::make_key(pk, sk);
            let items = self.items.lock().unwrap();
            Ok(items
                .get(&key)
                .map(|item| vec![item.iter().map(|(k, v)| (k.clone(), av_to_json(v))).collect()])
                .unwrap_or_default())
        }

        async fn query_gsi(&self, index_name: &str, gsi_pk: &str) -> Result<Vec<HashMap<String, Value>>, DynamoError> {
            let pk_attr = if index_name == "GSI2" { "GSI2PK" } else { "GSI1PK" };
            let items = self.items.lock().unwrap();
            let results: Vec<_> = items
                .values()
                .filter(|item| {
                    item.get(pk_attr)
                        .and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None })
                        == Some(gsi_pk)
                })
                .map(|item| item.iter().map(|(k, v)| (k.clone(), av_to_json(v))).collect())
                .collect();
            Ok(results)
        }

        async fn query_gsi_with_sk(
            &self,
            index_name: &str,
            gsi_pk: &str,
            gsi_sk: &str,
        ) -> Result<Vec<HashMap<String, Value>>, DynamoError> {
            let (pk_attr, sk_attr) = if index_name == "GSI2" {
                ("GSI2PK", "GSI2SK")
            } else {
                ("GSI1PK", "GSI1SK")
            };
            let items = self.items.lock().unwrap();
            let results: Vec<_> = items
                .values()
                .filter(|item| {
                    let pk_match = item
                        .get(pk_attr)
                        .and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None })
                        == Some(gsi_pk);
                    let sk_match = item
                        .get(sk_attr)
                        .and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None })
                        == Some(gsi_sk);
                    pk_match && sk_match
                })
                .map(|item| item.iter().map(|(k, v)| (k.clone(), av_to_json(v))).collect())
                .collect();
            Ok(results)
        }

        async fn delete_item(&self, pk: &str, sk: &str) -> Result<(), DynamoError> {
            let key = Self::make_key(pk, sk);
            self.items.lock().unwrap().remove(&key);
            Ok(())
        }

        async fn batch_delete_items(&self, items: &[KeyPair]) -> Result<(), DynamoError> {
            let mut store = self.items.lock().unwrap();
            for kp in items {
                let key = Self::make_key(&kp.pk, &kp.sk);
                store.remove(&key);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockDynamo;
    use super::*;

    #[tokio::test]
    async fn test_put_and_get_item() {
        let db = MockDynamo::new();
        let mut attrs = HashMap::new();
        attrs.insert("name".into(), Value::String("Alice".into()));

        db.put_item("user:1", "profile", "", "", "", "", attrs)
            .await
            .unwrap();

        let item = db.get_item("user:1", "profile").await.unwrap();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item["name"], Value::String("Alice".into()));
    }

    #[tokio::test]
    async fn test_get_item_not_found() {
        let db = MockDynamo::new();
        let item = db.get_item("nope", "nope").await.unwrap();
        assert!(item.is_none());
    }

    #[tokio::test]
    async fn test_query() {
        let db = MockDynamo::new();
        for i in 0..3 {
            db.put_item(
                "channel:user1",
                &format!("ch{}", i),
                "",
                "",
                "",
                "",
                HashMap::new(),
            )
            .await
            .unwrap();
        }
        // Different PK.
        db.put_item("channel:user2", "ch0", "", "", "", "", HashMap::new())
            .await
            .unwrap();

        let results = db.query("channel:user1").await.unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_query_with_sk() {
        let db = MockDynamo::new();
        db.put_item("otp:+1555", "123456", "", "", "", "", HashMap::new())
            .await
            .unwrap();
        db.put_item("otp:+1555", "999999", "", "", "", "", HashMap::new())
            .await
            .unwrap();

        let results = db.query_with_sk("otp:+1555", "123456").await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_query_gsi_with_sk() {
        let db = MockDynamo::new();
        db.put_item("channel:u1", "abc", "channel", "abc", "channel", "VEHICLES", HashMap::new())
            .await
            .unwrap();
        db.put_item("channel:u2", "def", "channel", "def", "channel", "FOOD", HashMap::new())
            .await
            .unwrap();

        let results = db.query_gsi_with_sk("GSI2", "channel", "VEHICLES").await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_delete_item() {
        let db = MockDynamo::new();
        let mut attrs = HashMap::new();
        attrs.insert("name".into(), Value::String("Bob".into()));
        db.put_item("user:1", "profile", "", "", "", "", attrs)
            .await
            .unwrap();

        db.delete_item("user:1", "profile").await.unwrap();
        let item = db.get_item("user:1", "profile").await.unwrap();
        assert!(item.is_none());
    }

    #[tokio::test]
    async fn test_batch_delete_items() {
        let db = MockDynamo::new();
        for i in 0..5 {
            db.put_item("sub:user1", &format!("ch{}", i), "", "", "", "", HashMap::new())
                .await
                .unwrap();
        }

        let pairs = vec![
            KeyPair { pk: "sub:user1".into(), sk: "ch0".into() },
            KeyPair { pk: "sub:user1".into(), sk: "ch1".into() },
            KeyPair { pk: "sub:user1".into(), sk: "ch2".into() },
        ];
        db.batch_delete_items(&pairs).await.unwrap();

        let remaining = db.query("sub:user1").await.unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[tokio::test]
    async fn test_batch_delete_empty() {
        let db = MockDynamo::new();
        db.batch_delete_items(&[]).await.unwrap();
    }

    #[tokio::test]
    async fn test_put_item_with_gsi() {
        let db = MockDynamo::new();
        let mut attrs = HashMap::new();
        attrs.insert("name".into(), Value::String("My Car Channel".into()));

        db.put_item("channel:u1", "abc123", "channel", "abc123", "channel", "VEHICLES", attrs)
            .await
            .unwrap();

        let results = db.query_gsi("GSI1", "channel").await.unwrap();
        assert_eq!(results.len(), 1);
    }
}

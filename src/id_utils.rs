use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Custom deserializer for converting various ID types to String
/// Handles both string and numeric inputs, converting everything to string
pub fn deserialize_id_as_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct IdAsStringVisitor;

    impl<'de> de::Visitor<'de> for IdAsStringVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or number representing an ID")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v.to_string())
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v.to_string())
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v.to_string())
        }

        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok((v as u64).to_string())
        }
    }

    deserializer.deserialize_any(IdAsStringVisitor)
}

/// Custom deserializer for converting optional ID types to Option<String>
pub fn deserialize_option_id_as_string<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionIdAsStringVisitor;

    impl<'de> de::Visitor<'de> for OptionIdAsStringVisitor {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an optional string or number representing an ID")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserialize_id_as_string(deserializer).map(Some)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(v.to_string()))
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(v.to_string()))
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(v.to_string()))
        }
    }

    deserializer.deserialize_option(OptionIdAsStringVisitor)
}

/// Custom serializer for String IDs (no conversion needed)
pub fn serialize_id_as_string<S>(value: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    value.serialize(serializer)
}

/// Custom serializer for Option<String> IDs 
pub fn serialize_option_id_as_string<S>(
    value: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(v) => serialize_id_as_string(v, serializer),
        None => serializer.serialize_none(),
    }
}

/// Helper function to convert any ID type to string
pub fn id_to_string<T: std::fmt::Display>(id: T) -> String {
    id.to_string()
}

/// Helper function to convert optional ID to Option<String>
pub fn option_id_to_string<T: std::fmt::Display>(opt_id: Option<T>) -> Option<String> {
    opt_id.map(|id| id.to_string())
}

/// Custom deserializer for Vec<String> that accepts both strings and numbers
pub fn deserialize_similar_artist_ids<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde_json::Value;
    
    let opt_value = Option::<Value>::deserialize(deserializer)?;
    match opt_value {
        Some(Value::Array(arr)) => {
            let mut result = Vec::new();
            for item in arr {
                match item {
                    Value::String(s) => result.push(s),
                    Value::Number(n) => {
                        if let Some(u) = n.as_u64() {
                            result.push(u.to_string());
                        } else if let Some(i) = n.as_i64() {
                            result.push(i.to_string());
                        } else if let Some(f) = n.as_f64() {
                            result.push((f as u64).to_string());
                        }
                    }
                    _ => continue,
                }
            }
            Ok(Some(result))
        }
        Some(_) => Ok(None),
        None => Ok(None),
    }
}

/// Custom serializer for Vec<String> to serialize as string array
pub fn serialize_similar_artist_ids<S>(
    value: &Option<Vec<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(vec) => vec.serialize(serializer),
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_deserialize_id_as_string_from_string() {
        #[derive(Debug, Deserialize)]
        struct Test {
            #[serde(deserialize_with = "deserialize_id_as_string")]
            id: String,
        }

        // Test string input
        let json_str = r#"{"id": "123"}"#;
        let result: Test = serde_json::from_str(json_str).unwrap();
        assert_eq!(result.id, "123");

        // Test number input (should be converted to string)
        let json_num = r#"{"id": 123}"#;
        let result: Test = serde_json::from_str(json_num).unwrap();
        assert_eq!(result.id, "123");
    }

    #[test]
    fn test_deserialize_option_id_as_string() {
        #[derive(Debug, Deserialize)]
        struct Test {
            #[serde(deserialize_with = "deserialize_option_id_as_string")]
            id: Option<String>,
        }

        // Test Some string input
        let json_str = r#"{"id": "123"}"#;
        let result: Test = serde_json::from_str(json_str).unwrap();
        assert_eq!(result.id, Some("123".to_string()));

        // Test Some number input (converted to string)
        let json_num = r#"{"id": 123}"#;
        let result: Test = serde_json::from_str(json_num).unwrap();
        assert_eq!(result.id, Some("123".to_string()));

        // Test None input
        let json_none = r#"{"id": null}"#;
        let result: Test = serde_json::from_str(json_none).unwrap();
        assert_eq!(result.id, None);
    }

    #[test]
    fn test_id_to_string() {
        assert_eq!(id_to_string("123"), "123");
        assert_eq!(id_to_string(456), "456");
        assert_eq!(id_to_string("abc123"), "abc123");
    }

    #[test]
    fn test_option_id_to_string() {
        assert_eq!(option_id_to_string(Some("123")), Some("123".to_string()));
        assert_eq!(option_id_to_string(Some(456)), Some("456".to_string()));
        assert_eq!(option_id_to_string::<String>(None), None);
    }

    #[test]
    fn test_mock_data_id_conversion() {
        // Test with mock discography data - now keeping IDs as strings
        let discography_json = r#"{
            "artist": {
                "id": 40226,
                "name": "Coldplay",
                "similarArtistIds": [35222, 118487, 40531, 71593, 45293]
            }
        }"#;
        
        // Parse as generic JSON first to see raw data
        let raw_value: serde_json::Value = serde_json::from_str(discography_json).unwrap();
        
        if let Some(artist) = raw_value.get("artist") {
            if let Some(id) = artist.get("id") {
                // Test our string conversion
                let id_num = id.as_u64().unwrap();
                assert_eq!(id_num, 40226);
                
                // Test string conversion function
                let id_str = id_to_string(id_num);
                assert_eq!(id_str, "40226");
            }
            
            if let Some(similar_ids) = artist.get("similarArtistIds") {
                if let Some(ids_array) = similar_ids.as_array() {
                    // Test conversion of each ID to string
                    for id in ids_array.iter() {
                        let id_num = id.as_u64().unwrap();
                        let id_str = id_to_string(id_num);
                        assert_eq!(id_str, id_num.to_string());
                    }
                }
            }
        }
    }

    #[test]
    fn test_album_id_conversion() {
        // Test album with string ID (like from API) - now keeping as string
        let album_json = r#"{
            "id": "0190295978044",
            "title": "Viva La Vida or Death and All His Friends",
            "artist": "Coldplay"
        }"#;
        
        let raw_value: serde_json::Value = serde_json::from_str(album_json).unwrap();
        
        if let Some(id) = raw_value.get("id") {
            let id_str = id.as_str().unwrap();
            
            // Test conversion - should keep as string
            let converted_id = id_to_string(id_str);
            assert_eq!(converted_id, "0190295978044");
            
            // Different strings should remain different
            let different_converted = id_to_string("different_string");
            assert_ne!(converted_id, different_converted);
        }
    }

    #[test]
    fn test_track_id_conversion() {
        // Test with numeric track IDs - now keeping as strings
        let track_ids = ["35541896", "35541897", "35541898"];
        
        for track_id in &track_ids {
            let converted_id = id_to_string(*track_id);
            
            // Should remain as string
            assert_eq!(converted_id, *track_id);
        }
    }
}
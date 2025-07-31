use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Custom deserializer for converting string IDs to u64
pub fn deserialize_u64_from_string<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    struct U64FromStringVisitor;

    impl<'de> de::Visitor<'de> for U64FromStringVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string containing a number or a number")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            // Try to parse as number first, fall back to hash if it fails
            Ok(v.parse().unwrap_or_else(|_| string_to_u64(v)))
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v)
        }
    }

    deserializer.deserialize_any(U64FromStringVisitor)
}

/// Custom deserializer for converting optional string IDs to Option<u64>
pub fn deserialize_option_u64_from_string<'de, D>(
    deserializer: D,
) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionU64FromStringVisitor;

    impl<'de> de::Visitor<'de> for OptionU64FromStringVisitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an optional string containing a number or a number")
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
            deserialize_u64_from_string(deserializer).map(Some)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_option(OptionU64FromStringVisitor)
}

/// Custom serializer for converting u64 to string (for API compatibility)
pub fn serialize_u64_as_string<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    value.to_string().serialize(serializer)
}

/// Custom serializer for converting Option<u64> to Option<String> (for API compatibility)
pub fn serialize_option_u64_as_string<S>(
    value: &Option<u64>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(v) => serialize_u64_as_string(v, serializer),
        None => serializer.serialize_none(),
    }
}

/// Helper function to convert string to u64 with fallback
pub fn string_to_u64(s: &str) -> u64 {
    s.parse().unwrap_or_else(|_| {
        // Fallback: use a hash of the string if it's not a number
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    })
}

/// Helper function to convert optional string to Option<u64>
pub fn option_string_to_u64(opt_s: &Option<String>) -> Option<u64> {
    opt_s.as_ref().map(|s| string_to_u64(s))
}

/// Custom deserializer for Vec<u64> that accepts both strings and numbers
pub fn deserialize_similar_artist_ids<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<u64>>, D::Error>
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
                    Value::String(s) => result.push(string_to_u64(&s)),
                    Value::Number(n) => {
                        if let Some(u) = n.as_u64() {
                            result.push(u);
                        } else if let Some(i) = n.as_i64() {
                            result.push(i as u64);
                        } else if let Some(f) = n.as_f64() {
                            result.push(f as u64);
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

/// Custom serializer for Vec<u64> to serialize as string array
pub fn serialize_similar_artist_ids<S>(
    value: &Option<Vec<u64>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(vec) => {
            let string_vec: Vec<String> = vec.iter().map(|id| id.to_string()).collect();
            string_vec.serialize(serializer)
        }
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_deserialize_u64_from_string() {
        #[derive(Debug, Deserialize)]
        struct Test {
            #[serde(deserialize_with = "deserialize_u64_from_string")]
            id: u64,
        }

        // Test string input
        let json_str = r#"{"id": "123"}"#;
        let result: Test = serde_json::from_str(json_str).unwrap();
        assert_eq!(result.id, 123);

        // Test number input
        let json_num = r#"{"id": 123}"#;
        let result: Test = serde_json::from_str(json_num).unwrap();
        assert_eq!(result.id, 123);
    }

    #[test]
    fn test_deserialize_option_u64_from_string() {
        #[derive(Debug, Deserialize)]
        struct Test {
            #[serde(deserialize_with = "deserialize_option_u64_from_string")]
            id: Option<u64>,
        }

        // Test Some string input
        let json_str = r#"{"id": "123"}"#;
        let result: Test = serde_json::from_str(json_str).unwrap();
        assert_eq!(result.id, Some(123));

        // Test Some number input
        let json_num = r#"{"id": 123}"#;
        let result: Test = serde_json::from_str(json_num).unwrap();
        assert_eq!(result.id, Some(123));

        // Test None input
        let json_none = r#"{"id": null}"#;
        let result: Test = serde_json::from_str(json_none).unwrap();
        assert_eq!(result.id, None);
    }

    #[test]
    fn test_string_to_u64() {
        assert_eq!(string_to_u64("123"), 123);
        assert_eq!(string_to_u64("456"), 456);
        
        // Test non-numeric string (should hash)
        let hash1 = string_to_u64("abc123");
        let hash2 = string_to_u64("abc123");
        assert_eq!(hash1, hash2); // Same input should produce same hash
        
        let hash3 = string_to_u64("def456");
        assert_ne!(hash1, hash3); // Different input should produce different hash
    }

    #[test]
    fn test_option_string_to_u64() {
        assert_eq!(option_string_to_u64(&Some("123".to_string())), Some(123));
        assert_eq!(option_string_to_u64(&Some("456".to_string())), Some(456));
        assert_eq!(option_string_to_u64(&None), None);
    }

    #[test]
    fn test_mock_data_id_conversion() {
        // Test with mock discography data 
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
                let id_str = id.to_string();
                let converted_id = string_to_u64(&id_str);
                assert_eq!(converted_id, 40226);
            }
            
            if let Some(similar_ids) = artist.get("similarArtistIds") {
                if let Some(ids_array) = similar_ids.as_array() {
                    // Test conversion of each ID
                    for id in ids_array.iter() {
                        let id_num = id.as_u64().unwrap();
                        let id_str = id.to_string();
                        let converted_id = string_to_u64(&id_str);
                        assert_eq!(converted_id, id_num);
                    }
                }
            }
        }
    }

    #[test]
    fn test_album_id_conversion() {
        // Test album with string ID (like from API)
        let album_json = r#"{
            "id": "0190295978044",
            "title": "Viva La Vida or Death and All His Friends",
            "artist": "Coldplay"
        }"#;
        
        let raw_value: serde_json::Value = serde_json::from_str(album_json).unwrap();
        
        if let Some(id) = raw_value.get("id") {
            let id_str = id.as_str().unwrap();
            
            // Test conversion - since this is not a pure number, it should hash
            let converted_id = string_to_u64(id_str);
            
            // Should produce a consistent hash
            let converted_id2 = string_to_u64(id_str);
            assert_eq!(converted_id, converted_id2);
            
            // Different strings should produce different hashes
            let different_converted = string_to_u64("different_string");
            assert_ne!(converted_id, different_converted);
        }
    }

    #[test]
    fn test_track_id_conversion() {
        // Test with numeric track IDs
        let track_ids = ["35541896", "35541897", "35541898"];
        
        for track_id in &track_ids {
            let converted_id = string_to_u64(track_id);
            
            // Should convert to the numeric value
            let expected: u64 = track_id.parse().unwrap();
            assert_eq!(converted_id, expected);
        }
    }
}
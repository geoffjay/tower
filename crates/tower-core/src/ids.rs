//! Strongly-typed ids (ULID strings underneath).

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug,
            Clone,
            PartialEq,
            Eq,
            Hash,
            serde::Serialize,
            serde::Deserialize,
            PartialOrd,
            Ord,
        )]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(crate::new_id())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
    };
}

id_type!(AgentId, "Agent identity (seat; stable across sessions)");
id_type!(TaskId, "Task identity");
id_type!(MessageId, "Message identity");
id_type!(MachineId, "Machine identity");

pub type EventSeq = i64;

//! Superset corpus verification (`A1`) and `Serialize` field-count (`C2`) tests for
//! [`league_error`].

use league_error::{StructuredError, scrub_secret_text, scrub_secret_text_with_marker};
use serde::Serialize;
use serde::ser::{
    SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};
use serde_json::Value;
use std::fmt;

#[test]
#[allow(clippy::disallowed_macros)]
fn a1_shared_scrubber_catches_union_of_all_old_copies_plus_a1_b3_c1() {
    let raw = include_str!("scrubber_superset_corpus.json");
    let entries: Vec<Value> =
        serde_json::from_str(raw).unwrap_or_else(|e| panic!("invalid corpus json: {e}"));
    assert_eq!(entries.len(), 57, "expected all 57 shared corpus entries");

    for entry in &entries {
        let id = entry["id"].as_str().unwrap_or("");
        let input = entry["input"].as_str().unwrap_or("");
        let expect_unchanged = entry["expect_unchanged"].as_bool().unwrap_or(false);
        let needles: Vec<&str> = entry["secret_needles"]
            .as_array()
            .map(|arr| arr.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let any_old = entry["redacted_by"]["core_7fd3b481"]
            .as_bool()
            .unwrap_or(false)
            || entry["redacted_by"]["ledger_9818e991"]
                .as_bool()
                .unwrap_or(false)
            || entry["redacted_by"]["relay_9818e991"]
                .as_bool()
                .unwrap_or(false);

        let default_out = scrub_secret_text(input);
        let relay_out = scrub_secret_text_with_marker(input, "<REDACTED_API_KEY>");

        if expect_unchanged {
            assert_eq!(default_out, input, "benign corpus entry '{id}' changed");
            assert_eq!(relay_out, input, "benign corpus entry '{id}' changed");
            continue;
        }

        // Unconditionally assert every secret-bearing corpus entry — and every input any old copy
        // redacted — is scrubbed by both markers without leaving any needle behind.
        assert!(
            !needles.is_empty(),
            "corpus entry '{id}' must list secret_needles"
        );
        for (mode, scrubbed) in [("default", &default_out), ("relay", &relay_out)] {
            assert_ne!(
                scrubbed, input,
                "shared scrubber ({mode}) failed to redact '{id}' (any_old={any_old}): {input:?}"
            );
            for needle in &needles {
                assert!(
                    !scrubbed.contains(needle),
                    "shared scrubber ({mode}) leaked '{needle}' on '{id}' (any_old={any_old}): {scrubbed:?}"
                );
            }
            assert_eq!(
                scrub_secret_text(scrubbed),
                *scrubbed,
                "shared scrubber ({mode}) not idempotent on '{id}': {scrubbed:?}"
            );
        }
    }
}

#[derive(Debug)]
struct SpyErr(String);

impl fmt::Display for SpyErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SpyErr {}

impl serde::ser::Error for SpyErr {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

struct FieldCountSpy {
    struct_name: Option<&'static str>,
    struct_len: Option<usize>,
    serialized_fields: Vec<&'static str>,
    skipped_fields: Vec<&'static str>,
}

struct SpyStruct<'a>(&'a mut FieldCountSpy);

impl SerializeStruct for SpyStruct<'_> {
    type Ok = ();
    type Error = SpyErr;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        _value: &T,
    ) -> Result<(), Self::Error> {
        self.0.serialized_fields.push(key);
        Ok(())
    }

    fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
        self.0.skipped_fields.push(key);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

struct NopCompound;

impl SerializeSeq for NopCompound {
    type Ok = ();
    type Error = SpyErr;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), SpyErr> {
        Ok(())
    }
    fn end(self) -> Result<(), SpyErr> {
        Ok(())
    }
}

impl SerializeTuple for NopCompound {
    type Ok = ();
    type Error = SpyErr;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), SpyErr> {
        Ok(())
    }
    fn end(self) -> Result<(), SpyErr> {
        Ok(())
    }
}

impl SerializeTupleStruct for NopCompound {
    type Ok = ();
    type Error = SpyErr;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), SpyErr> {
        Ok(())
    }
    fn end(self) -> Result<(), SpyErr> {
        Ok(())
    }
}

impl SerializeTupleVariant for NopCompound {
    type Ok = ();
    type Error = SpyErr;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), SpyErr> {
        Ok(())
    }
    fn end(self) -> Result<(), SpyErr> {
        Ok(())
    }
}

impl SerializeMap for NopCompound {
    type Ok = ();
    type Error = SpyErr;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), SpyErr> {
        Ok(())
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), SpyErr> {
        Ok(())
    }
    fn end(self) -> Result<(), SpyErr> {
        Ok(())
    }
}

impl SerializeStructVariant for NopCompound {
    type Ok = ();
    type Error = SpyErr;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _: &'static str,
        _: &T,
    ) -> Result<(), SpyErr> {
        Ok(())
    }
    fn end(self) -> Result<(), SpyErr> {
        Ok(())
    }
}

macro_rules! unsupported_ser {
    ($fn_name:ident ($($arg:ident : $ty:ty),*)) => {
        fn $fn_name(self, $(_: $ty),*) -> Result<Self::Ok, Self::Error> {
            Err(SpyErr("unsupported".to_owned()))
        }
    };
}

impl<'a> Serializer for &'a mut FieldCountSpy {
    type Ok = ();
    type Error = SpyErr;
    type SerializeSeq = NopCompound;
    type SerializeTuple = NopCompound;
    type SerializeTupleStruct = NopCompound;
    type SerializeTupleVariant = NopCompound;
    type SerializeMap = NopCompound;
    type SerializeStruct = SpyStruct<'a>;
    type SerializeStructVariant = NopCompound;

    unsupported_ser!(serialize_bool(v: bool));
    unsupported_ser!(serialize_i8(v: i8));
    unsupported_ser!(serialize_i16(v: i16));
    unsupported_ser!(serialize_i32(v: i32));
    unsupported_ser!(serialize_i64(v: i64));
    unsupported_ser!(serialize_u8(v: u8));
    unsupported_ser!(serialize_u16(v: u16));
    unsupported_ser!(serialize_u32(v: u32));
    unsupported_ser!(serialize_u64(v: u64));
    unsupported_ser!(serialize_f32(v: f32));
    unsupported_ser!(serialize_f64(v: f64));
    unsupported_ser!(serialize_char(v: char));
    unsupported_ser!(serialize_str(v: &str));
    unsupported_ser!(serialize_bytes(v: &[u8]));

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(SpyErr("unsupported".to_owned()))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, _: &T) -> Result<Self::Ok, Self::Error> {
        Err(SpyErr("unsupported".to_owned()))
    }
    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(SpyErr("unsupported".to_owned()))
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(SpyErr("unsupported".to_owned()))
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Err(SpyErr("unsupported".to_owned()))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(SpyErr("unsupported".to_owned()))
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(SpyErr("unsupported".to_owned()))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(NopCompound)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(NopCompound)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(NopCompound)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(NopCompound)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(NopCompound)
    }
    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.struct_name = Some(name);
        self.struct_len = Some(len);
        Ok(SpyStruct(self))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(NopCompound)
    }
}

#[test]
fn c2_structured_error_serialize_always_declares_5_fields_and_skips_none_cause() {
    let without_cause = StructuredError::new("E_ONE", "msg", "hint", false);
    let mut spy_none = FieldCountSpy {
        struct_name: None,
        struct_len: None,
        serialized_fields: Vec::new(),
        skipped_fields: Vec::new(),
    };
    if let Err(err) = without_cause.serialize(&mut spy_none) {
        panic!("spy_none serialize failed: {err}");
    }
    assert_eq!(spy_none.struct_name, Some("StructuredError"));
    assert_eq!(spy_none.struct_len, Some(5));
    assert_eq!(
        spy_none.serialized_fields,
        vec!["code", "message", "hint", "retryable"]
    );
    assert_eq!(spy_none.skipped_fields, vec!["cause"]);

    let with_cause = without_cause.with_cause("root cause");
    let mut spy_some = FieldCountSpy {
        struct_name: None,
        struct_len: None,
        serialized_fields: Vec::new(),
        skipped_fields: Vec::new(),
    };
    if let Err(err) = with_cause.serialize(&mut spy_some) {
        panic!("spy_some serialize failed: {err}");
    }
    assert_eq!(spy_some.struct_name, Some("StructuredError"));
    assert_eq!(spy_some.struct_len, Some(5));
    assert_eq!(
        spy_some.serialized_fields,
        vec!["code", "message", "hint", "cause", "retryable"]
    );
    assert!(spy_some.skipped_fields.is_empty());
}

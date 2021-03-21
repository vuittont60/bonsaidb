use std::marker::PhantomData;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::schema::{Collection, Map, Revision};

/// a struct representing a document in the database
pub struct Document<C> {
    /// the id of the Document. Unique across the collection `C`
    pub id: Uuid,
    /// the revision of the stored document.
    pub revision: Revision,

    /// the serialized bytes of the stored item
    // TODO this should be a Cow to allow deserialization without copying
    pub contents: Vec<u8>,

    _collection: PhantomData<C>,
}

impl<C> Document<C>
where
    C: Collection,
{
    /// create a new document with serialized bytes from `contents`
    pub fn new<S: Serialize>(contents: &S) -> Result<Self, serde_cbor::Error> {
        let contents = serde_cbor::to_vec(contents)?;
        let revision = Revision::new(&contents);
        Ok(Self {
            id: Uuid::new_v4(),
            revision,
            contents,
            _collection: PhantomData::default(),
        })
    }

    /// retrieves `contents` through deserialization into the type `D`
    pub fn contents<'a, D: Deserialize<'a>>(&'a self) -> Result<D, serde_cbor::Error> {
        serde_cbor::from_slice(&self.contents)
    }

    pub(crate) fn update_with<S: Serialize>(
        &self,
        contents: &S,
    ) -> Result<Option<Self>, serde_cbor::Error> {
        let contents = serde_cbor::to_vec(contents)?;
        Ok(self.revision.next_revision(&contents).map(|revision| Self {
            id: self.id,
            revision,
            contents,
            _collection: PhantomData::default(),
        }))
    }

    /// create a `Map` result with an empty key and value
    #[must_use]
    pub fn emit(&self) -> Map<(), ()> {
        self.emit_key_and_value((), ())
    }

    /// create a `Map` result with a `key` and an empty value
    #[must_use]
    pub fn emit_key<Key: Serialize>(&self, key: Key) -> Map<Key, ()> {
        self.emit_key_and_value(key, ())
    }

    /// create a `Map` result with `value` and an empty key
    #[must_use]
    pub fn emit_value<Value: Serialize>(&self, value: Value) -> Map<(), Value> {
        self.emit_key_and_value((), value)
    }

    /// create a `Map` result with a `key` and `value`
    #[must_use]
    pub fn emit_key_and_value<Key: Serialize, Value: Serialize>(
        &self,
        key: Key,
        value: Value,
    ) -> Map<Key, Value> {
        Map {
            source: self.id,
            key,
            value,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        connection::Connection,
        schema::Map,
        storage::Storage,
        test_util::{Basic, BasicCollection},
        Error,
    };

    use super::Document;

    #[tokio::test]
    #[ignore] // TODO make this test work
    async fn store_retrieve() -> Result<(), Error> {
        let path = std::env::temp_dir().join("store_retrieve_tests.pliantdb");
        if path.exists() {
            std::fs::remove_dir_all(&path).unwrap();
        }
        let db = Storage::<BasicCollection>::open_local(path)?;

        let original_value = Basic { parent_id: None };
        let doc = db
            .collection::<BasicCollection>()?
            .push(&original_value)
            .await?;

        let doc = db
            .collection::<BasicCollection>()?
            .get(&doc.id)
            .await?
            .expect("couldn't retrieve stored item");

        assert_eq!(original_value, doc.contents::<Basic>()?);

        Ok(())
    }

    #[test]
    fn emissions() -> Result<(), Error> {
        let doc = Document::<BasicCollection>::new(&Basic { parent_id: None })?;

        assert_eq!(
            doc.emit(),
            Map {
                source: doc.id,
                key: (),
                value: ()
            }
        );

        assert_eq!(
            doc.emit_key(1),
            Map {
                source: doc.id,
                key: 1,
                value: ()
            }
        );

        assert_eq!(
            doc.emit_value(1),
            Map {
                source: doc.id,
                key: (),
                value: 1
            }
        );

        assert_eq!(
            doc.emit_key_and_value(1, 2),
            Map {
                source: doc.id,
                key: 1,
                value: 2
            }
        );

        Ok(())
    }
}

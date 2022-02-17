use std::{borrow::Cow, fmt::Debug, marker::PhantomData, task::Poll};

use async_trait::async_trait;
use futures::{future::BoxFuture, ready, Future, FutureExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use transmog::{Format, OwnedDeserializer};
use transmog_pot::Pot;

use crate::{
    connection::{self, Connection, Range},
    document::{
        BorrowedDocument, CollectionDocument, Document, DocumentId, DocumentKey, KeyId,
        OwnedDocument, OwnedDocuments,
    },
    schema::{view::Key, CollectionName, Schematic},
    Error,
};

/// A namespaced collection of `Document<Self>` items and views.
///
/// ## Deriving this trait
///
/// This trait can be derived instead of manually implemented:
///
/// ```rust
/// use bonsaidb_core::schema::Collection;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Serialize, Deserialize, Default, Collection)]
/// #[collection(name = "MyCollection")]
/// # #[collection(core = bonsaidb_core)]
/// pub struct MyCollection;
/// ```
///
/// If you're publishing a collection for use in multiple projects, consider
/// giving the collection an `authority`, which gives your collection a
/// namespace:
///
/// ```rust
/// use bonsaidb_core::schema::Collection;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Serialize, Deserialize, Default, Collection)]
/// #[collection(name = "MyCollection", authority = "khonsulabs")]
/// # #[collection(core = bonsaidb_core)]
/// pub struct MyCollection;
/// ```
///
/// The list of views can be specified using the `views` parameter:
///
/// ```rust
/// use bonsaidb_core::schema::{Collection, View};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Debug, Serialize, Deserialize, Default, Collection)]
/// #[collection(name = "MyCollection", views = [ScoresByRank])]
/// # #[collection(core = bonsaidb_core)]
/// pub struct MyCollection;
///
/// #[derive(Debug, Clone, View)]
/// #[view(collection = MyCollection, key = u32, value = f32, name = "scores-by-rank")]
/// # #[view(core = bonsaidb_core)]
/// pub struct ScoresByRank;
/// #
/// # use bonsaidb_core::{
/// #     document::CollectionDocument,
/// #     schema::{
/// #         CollectionViewSchema,   ReduceResult,
/// #         ViewMapResult, ViewMappedValue,
/// #    },
/// # };
/// # impl CollectionViewSchema for ScoresByRank {
/// #     type View = Self;
/// #     fn map(
/// #         &self,
/// #         _document: CollectionDocument<<Self::View as View>::Collection>,
/// #     ) -> ViewMapResult<Self::View> {
/// #         todo!()
/// #     }
/// #
/// #     fn reduce(
/// #         &self,
/// #         _mappings: &[ViewMappedValue<Self::View>],
/// #         _rereduce: bool,
/// #     ) -> ReduceResult<Self::View> {
/// #         todo!()
/// #     }
/// # }
/// ```
///
/// ### Specifying a Collection Encryption Key
///
/// By default, encryption will be required if an `encryption_key` is provided:
///
/// ```rust
/// use bonsaidb_core::{document::KeyId, schema::Collection};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Serialize, Deserialize, Default, Collection)]
/// #[collection(name = "MyCollection", encryption_key = Some(KeyId::Master))]
/// # #[collection(core = bonsaidb_core)]
/// pub struct MyCollection;
/// ```
///
/// The `encryption_required` parameter can be provided if you wish to be
/// explicit:
///
/// ```rust
/// use bonsaidb_core::{document::KeyId, schema::Collection};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Serialize, Deserialize, Default, Collection)]
/// #[collection(name = "MyCollection")]
/// #[collection(encryption_key = Some(KeyId::Master), encryption_required)]
/// # #[collection(core = bonsaidb_core)]
/// pub struct MyCollection;
/// ```
///
/// Or, if you wish your collection to be encrypted if its available, but not
/// cause errors when being stored without encryption, you can provide the
/// `encryption_optional` parameter:
///
/// ```rust
/// use bonsaidb_core::{document::KeyId, schema::Collection};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Serialize, Deserialize, Default, Collection)]
/// #[collection(name = "MyCollection")]
/// #[collection(encryption_key = Some(KeyId::Master), encryption_optional)]
/// # #[collection(core = bonsaidb_core)]
/// pub struct MyCollection;
/// ```
///
/// ### Changing the serialization strategy
///
/// BonsaiDb uses [`transmog`](::transmog) to allow customizing serialization
/// formats. To use one of the formats Transmog already supports, add its crate
/// to your Cargo.toml and use it like this example using `transmog_bincode`:
///
/// ```rust
/// use bonsaidb_core::schema::Collection;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Serialize, Deserialize, Default, Collection)]
/// #[collection(name = "MyCollection")]
/// #[collection(serialization = transmog_bincode::Bincode)]
/// # #[collection(core = bonsaidb_core)]
/// pub struct MyCollection;
/// ```
///
/// To manually implement `SerializedCollection` you can pass `None` to
/// `serialization`:
///
/// ```rust
/// use bonsaidb_core::schema::Collection;
///
/// #[derive(Debug, Default, Collection)]
/// #[collection(name = "MyCollection")]
/// #[collection(serialization = None)]
/// # #[collection(core = bonsaidb_core)]
/// pub struct MyCollection;
/// ```
pub trait Collection: Debug + Send + Sync
where
    DocumentKey<Self::PrimaryKey>: From<Self::PrimaryKey>,
{
    /// The unique id type. Each document stored in a collection will be
    /// uniquely identified by this type.
    type PrimaryKey: for<'k> Key<'k>;

    /// The unique name of this collection. Each collection must be uniquely
    /// named within the [`Schema`](crate::schema::Schema) it is registered
    /// within.
    fn collection_name() -> CollectionName;

    /// Defines all `View`s in this collection in `schema`.
    fn define_views(schema: &mut Schematic) -> Result<(), Error>;

    /// If a [`KeyId`] is returned, this collection will be stored encrypted
    /// at-rest using the key specified.
    #[must_use]
    fn encryption_key() -> Option<KeyId> {
        None
    }
}

/// A collection that knows how to serialize and deserialize documents to an associated type.
///
/// These examples for this type use this basic collection definition:
///
/// ```rust
/// use bonsaidb_core::{
///     schema::{Collection, CollectionName, DefaultSerialization, Schematic},
///     Error,
/// };
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Serialize, Deserialize, Default, Collection)]
/// #[collection(name = "MyCollection")]
/// # #[collection(core = bonsaidb_core)]
/// pub struct MyCollection {
///     pub rank: u32,
///     pub score: f32,
/// }
/// ```
#[async_trait]
pub trait SerializedCollection: Collection {
    /// The type of the contents stored in documents in this collection.
    type Contents: Send + Sync;
    /// The serialization format for this collection.
    type Format: OwnedDeserializer<Self::Contents>;

    /// Returns the configured instance of [`Self::Format`].
    // TODO allow configuration to be passed here, such as max allocation bytes.
    fn format() -> Self::Format;

    /// Deserialize `data` as `Self::Contents` using this collection's format.
    fn deserialize(data: &[u8]) -> Result<Self::Contents, Error> {
        Self::format()
            .deserialize_owned(data)
            .map_err(|err| crate::Error::Serialization(err.to_string()))
    }

    /// Returns the deserialized contents of `doc`.
    fn document_contents<D: Document<Self>>(doc: &D) -> Result<Self::Contents, Error>
    where
        Self::Contents: Clone,
        Self: Sized,
    {
        doc.contents()
    }

    /// Sets the contents of `doc` to `contents`.
    fn set_document_contents<D: Document<Self>>(
        doc: &mut D,
        contents: Self::Contents,
    ) -> Result<(), Error>
    where
        Self: Sized,
    {
        doc.set_contents(contents)
    }

    /// Serialize `item` using this collection's format.
    fn serialize(item: &Self::Contents) -> Result<Vec<u8>, Error> {
        Self::format()
            .serialize(item)
            .map_err(|err| crate::Error::Serialization(err.to_string()))
    }

    /// Gets a [`CollectionDocument`] with `id` from `connection`.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// if let Some(doc) = MyCollection::get(42, &db).await? {
    ///     println!(
    ///         "Retrieved revision {} with deserialized contents: {:?}",
    ///         doc.header.revision, doc.contents
    ///     );
    /// }
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    async fn get<C, PK>(id: PK, connection: &C) -> Result<Option<CollectionDocument<Self>>, Error>
    where
        C: Connection,
        PK: Into<DocumentKey<Self::PrimaryKey>> + Send,
        Self: Sized,
    {
        let possible_doc = connection.get::<Self, _>(id).await?;
        Ok(possible_doc.as_ref().map(TryInto::try_into).transpose()?)
    }

    /// Retrieves all documents matching `ids`. Documents that are not found
    /// are not returned, but no error will be generated.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// for doc in MyCollection::get_multiple([42, 43], &db).await? {
    ///     println!(
    ///         "Retrieved #{} with deserialized contents: {:?}",
    ///         doc.header.id, doc.contents
    ///     );
    /// }
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    async fn get_multiple<C, DocumentIds, PK, I>(
        ids: DocumentIds,
        connection: &C,
    ) -> Result<Vec<CollectionDocument<Self>>, Error>
    where
        C: Connection,
        DocumentIds: IntoIterator<Item = PK, IntoIter = I> + Send + Sync,
        I: Iterator<Item = PK> + Send + Sync,
        PK: Into<DocumentKey<Self::PrimaryKey>> + Send + Sync,
        Self: Sized,
    {
        connection
            .collection::<Self>()
            .get_multiple(ids)
            .await
            .and_then(|docs| docs.collection_documents())
    }

    /// Retrieves all documents matching the range of `ids`.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// for doc in MyCollection::list(42.., &db).descending().limit(20).await? {
    ///     println!(
    ///         "Retrieved #{} with deserialized contents: {:?}",
    ///         doc.header.id, doc.contents
    ///     );
    /// }
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    fn list<R, PK, C>(ids: R, connection: &'_ C) -> List<'_, C, Self>
    where
        R: Into<Range<PK>>,
        C: Connection,
        PK: Into<DocumentKey<Self::PrimaryKey>> + Send + Sync,
        Self: Sized,
    {
        List(connection::List::new(
            connection::PossiblyOwned::Owned(connection.collection::<Self>()),
            ids.into().map(PK::into),
        ))
    }

    /// Retrieves all documents.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// for doc in MyCollection::all(&db).await? {
    ///     println!(
    ///         "Retrieved #{} with deserialized contents: {:?}",
    ///         doc.header.id, doc.contents
    ///     );
    /// }
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    fn all<C: Connection>(connection: &C) -> List<'_, C, Self>
    where
        Self: Sized,
    {
        List(connection::List::new(
            connection::PossiblyOwned::Owned(connection.collection::<Self>()),
            Range::from(..),
        ))
    }

    /// Pushes this value into the collection, returning the created document.
    /// This function is useful when `Self != Self::Contents`.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let document = MyCollection::push(MyCollection::default(), &db).await?;
    /// println!(
    ///     "Inserted {:?} with id {} with revision {}",
    ///     document.contents, document.header.id, document.header.revision
    /// );
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    async fn push<Cn: Connection>(
        contents: Self::Contents,
        connection: &Cn,
    ) -> Result<CollectionDocument<Self>, InsertError<Self::Contents>>
    where
        Self: Sized + 'static,
        Self::Contents: 'async_trait,
    {
        let header = match connection.collection::<Self>().push(&contents).await {
            Ok(header) => header,
            Err(error) => return Err(InsertError { contents, error }),
        };
        Ok(CollectionDocument { header, contents })
    }

    /// Pushes this value into the collection, returning the created document.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let document = MyCollection::default().push_into(&db).await?;
    /// println!(
    ///     "Inserted {:?} with id {} with revision {}",
    ///     document.contents, document.header.id, document.header.revision
    /// );
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    async fn push_into<Cn: Connection>(
        self,
        connection: &Cn,
    ) -> Result<CollectionDocument<Self>, InsertError<Self>>
    where
        Self: SerializedCollection<Contents = Self> + Sized + 'static,
    {
        Self::push(self, connection).await
    }

    /// Inserts this value into the collection with the specified id, returning
    /// the created document.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let document = MyCollection::insert(42, MyCollection::default(), &db).await?;
    /// assert_eq!(document.header.id, 42);
    /// println!(
    ///     "Inserted {:?} with revision {}",
    ///     document.contents, document.header.revision
    /// );
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    async fn insert<PK, Cn>(
        id: PK,
        contents: Self::Contents,
        connection: &Cn,
    ) -> Result<CollectionDocument<Self>, InsertError<Self::Contents>>
    where
        PK: Into<DocumentKey<Self::PrimaryKey>> + Send + Sync,
        Cn: Connection,
        Self: Sized + 'static,
        Self::Contents: 'async_trait,
    {
        let header = match connection.collection::<Self>().insert(id, &contents).await {
            Ok(header) => header,
            Err(error) => return Err(InsertError { contents, error }),
        };
        Ok(CollectionDocument { header, contents })
    }

    /// Inserts this value into the collection with the given `id`, returning
    /// the created document.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let document = MyCollection::default().insert_into(42, &db).await?;
    /// assert_eq!(document.header.id, 42);
    /// println!(
    ///     "Inserted {:?} with revision {}",
    ///     document.contents, document.header.revision
    /// );
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    async fn insert_into<PK, Cn>(
        self,
        id: PK,
        connection: &Cn,
    ) -> Result<CollectionDocument<Self>, InsertError<Self>>
    where
        PK: Into<DocumentKey<Self::PrimaryKey>> + Send + Sync,
        Cn: Connection,
        Self: SerializedCollection<Contents = Self> + Sized + 'static,
    {
        Self::insert(id, self, connection).await
    }

    /// Overwrites this value into the collection with the specified id, returning
    /// the created or updated document.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let document = MyCollection::overwrite(42, MyCollection::default(), &db).await?;
    /// assert_eq!(document.header.id, 42);
    /// println!(
    ///     "Overwrote {:?} with revision {}",
    ///     document.contents, document.header.revision
    /// );
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    async fn overwrite<PK, Cn>(
        id: PK,
        contents: Self::Contents,
        connection: &Cn,
    ) -> Result<CollectionDocument<Self>, InsertError<Self::Contents>>
    where
        PK: Into<DocumentKey<Self::PrimaryKey>> + Send,
        Cn: Connection,
        Self: Sized + 'static,
        Self::Contents: 'async_trait,
    {
        let header = match Self::serialize(&contents) {
            Ok(serialized) => match connection.overwrite::<Self, _>(id, serialized).await {
                Ok(header) => header,
                Err(error) => return Err(InsertError { contents, error }),
            },
            Err(error) => return Err(InsertError { contents, error }),
        };
        Ok(CollectionDocument { header, contents })
    }

    /// Overwrites this value into the collection with the given `id`, returning
    /// the created or updated document.
    ///
    /// ```rust
    /// # bonsaidb_core::__doctest_prelude!();
    /// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let document = MyCollection::default().overwrite_into(42, &db).await?;
    /// assert_eq!(document.header.id, 42);
    /// println!(
    ///     "Overwrote {:?} with revision {}",
    ///     document.contents, document.header.revision
    /// );
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    async fn overwrite_into<Cn: Connection, PK>(
        self,
        id: PK,
        connection: &Cn,
    ) -> Result<CollectionDocument<Self>, InsertError<Self>>
    where
        PK: Into<DocumentKey<Self::PrimaryKey>> + Send + Sync,
        Self: SerializedCollection<Contents = Self> + Sized + 'static,
    {
        Self::overwrite(id, self, connection).await
    }
}

/// A convenience trait for easily storing Serde-compatible types in documents.
pub trait DefaultSerialization: Collection {}

impl<T> SerializedCollection for T
where
    T: DefaultSerialization + Clone + Serialize + DeserializeOwned,
{
    type Contents = Self;
    type Format = Pot;

    fn format() -> Self::Format {
        Pot::default()
    }
}

/// An error from inserting a [`CollectionDocument`].
#[derive(thiserror::Error, Debug)]
#[error("{error}")]
pub struct InsertError<T> {
    /// The original value being inserted.
    pub contents: T,
    /// The error that occurred while inserting.
    pub error: Error,
}

/// A collection with a unique name column.
///
/// ## Finding a document by unique name
///
/// ```rust
/// # bonsaidb_core::__doctest_prelude!();
/// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// if let Some(doc) = MyCollection::load("unique name", &db).await? {
///     println!(
///         "Retrieved revision {} with deserialized contents: {:?}",
///         doc.header.revision, doc.contents
///     );
/// }
/// # Ok(())
/// # })
/// # }
/// ```
///
/// Load accepts either a string or a [`DocumentId`]. This enables building
/// methods that accept either the unique ID or the unique name:
///
/// ```rust
/// # bonsaidb_core::__doctest_prelude!();
/// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// if let Some(doc) = MyCollection::load(42, &db).await? {
///     println!(
///         "Retrieved revision {} with deserialized contents: {:?}",
///         doc.header.revision, doc.contents
///     );
/// }
/// # Ok(())
/// # })
/// # }
/// ```
///
/// ## Executing an insert or update
///
/// ```rust
/// # bonsaidb_core::__doctest_prelude!();
/// # fn test_fn<C: Connection>(db: C) -> Result<(), Error> {
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let upserted = MyCollection::entry("unique name", &db)
///     .update_with(|existing: &mut MyCollection| {
///         existing.rank += 1;
///     })
///     .or_insert_with(MyCollection::default)
///     .await?
///     .unwrap();
/// println!("Rank: {:?}", upserted.contents.rank);
///
/// # Ok(())
/// # })
/// # }
/// ```
#[async_trait]
pub trait NamedCollection: Collection + Unpin {
    /// The name view defined for the collection.
    type ByNameView: crate::schema::SerializedView<Key = String>;

    /// Gets a [`CollectionDocument`] with `id` from `connection`.
    async fn load<'name, N: Nameable<'name, Self::PrimaryKey> + Send + Sync, C: Connection>(
        id: N,
        connection: &C,
    ) -> Result<Option<CollectionDocument<Self>>, Error>
    where
        Self: SerializedCollection + Sized + 'static,
    {
        let possible_doc = Self::load_document(id, connection).await?;
        Ok(possible_doc
            .as_ref()
            .map(CollectionDocument::try_from)
            .transpose()?)
    }

    /// Gets a [`CollectionDocument`] with `id` from `connection`.
    fn entry<
        'connection,
        'name,
        N: Into<NamedReference<'name, Self::PrimaryKey>> + Send + Sync,
        C: Connection,
    >(
        id: N,
        connection: &'connection C,
    ) -> Entry<'connection, 'name, C, Self, (), ()>
    where
        Self: SerializedCollection + Sized,
    {
        let name = id.into();
        Entry {
            state: EntryState::Pending(Some(EntryBuilder {
                name,
                connection,
                insert: None,
                update: None,
                retry_limit: 0,
                _collection: PhantomData,
            })),
        }
    }

    /// Loads a document from this collection by name, if applicable. Return
    /// `Ok(None)` if unsupported.
    #[allow(unused_variables)]
    async fn load_document<
        'name,
        N: Nameable<'name, Self::PrimaryKey> + Send + Sync,
        C: Connection,
    >(
        name: N,
        connection: &C,
    ) -> Result<Option<OwnedDocument>, Error>
    where
        Self: SerializedCollection + Sized,
    {
        match name.name()? {
            NamedReference::Id(id) => connection.collection::<Self>().get(id).await,
            NamedReference::Key(id) => connection.collection::<Self>().get(id).await,
            NamedReference::Name(name) => Ok(connection
                .view::<Self::ByNameView>()
                .with_key(name.as_ref().to_owned())
                .query_with_docs()
                .await?
                .documents
                .into_iter()
                .next()
                .map(|(_, document)| document)),
        }
    }
}

/// A reference to a collection that has a unique name view.
#[derive(Clone, PartialEq, Deserialize, Serialize, Debug)]
#[must_use]
pub enum NamedReference<'a, Id> {
    /// An entity's name.
    Name(Cow<'a, str>),
    /// A document id.
    Id(DocumentId),
    /// A document id.
    Key(Id),
}

impl<'a, Id> From<&'a str> for NamedReference<'a, Id> {
    fn from(name: &'a str) -> Self {
        Self::Name(Cow::Borrowed(name))
    }
}

/// A type that can be used as a unique reference for a collection that
/// implements [`NamedCollection`].
pub trait Nameable<'a, Id> {
    /// Returns this name as a [`NamedReference`].
    fn name(self) -> Result<NamedReference<'a, Id>, crate::Error>;
}

impl<'a, Id> Nameable<'a, Id> for NamedReference<'a, Id> {
    fn name(self) -> Result<NamedReference<'a, Id>, crate::Error> {
        Ok(self)
    }
}

impl<'a, Id> Nameable<'a, Id> for &'a str {
    fn name(self) -> Result<NamedReference<'a, Id>, crate::Error> {
        Ok(NamedReference::from(self))
    }
}

impl<'a, Id> From<&'a String> for NamedReference<'a, Id> {
    fn from(name: &'a String) -> Self {
        Self::Name(Cow::Borrowed(name.as_str()))
    }
}

impl<'a, Id> Nameable<'a, Id> for &'a String {
    fn name(self) -> Result<NamedReference<'a, Id>, crate::Error> {
        Ok(NamedReference::from(self))
    }
}

impl<'a, 'b, 'c, Id> From<&'b BorrowedDocument<'b>> for NamedReference<'a, Id> {
    fn from(doc: &'b BorrowedDocument<'b>) -> Self {
        Self::Id(doc.header.id)
    }
}

impl<'a, 'b, Id> Nameable<'a, Id> for &'a BorrowedDocument<'b> {
    fn name(self) -> Result<NamedReference<'a, Id>, crate::Error> {
        Ok(NamedReference::from(self))
    }
}

impl<'a, 'c, C> TryFrom<&'c CollectionDocument<C>> for NamedReference<'a, C::PrimaryKey>
where
    C: SerializedCollection,
{
    type Error = crate::Error;

    fn try_from(doc: &'c CollectionDocument<C>) -> Result<Self, crate::Error> {
        DocumentId::new(doc.header.id.clone()).map(Self::Id)
    }
}

impl<'a, C> Nameable<'a, C::PrimaryKey> for &'a CollectionDocument<C>
where
    C: SerializedCollection,
{
    fn name(self) -> Result<NamedReference<'a, C::PrimaryKey>, crate::Error> {
        NamedReference::try_from(self)
    }
}

impl<'a, Id> From<String> for NamedReference<'a, Id> {
    fn from(name: String) -> Self {
        Self::Name(Cow::Owned(name))
    }
}

impl<'a, Id> Nameable<'a, Id> for String {
    fn name(self) -> Result<NamedReference<'a, Id>, crate::Error> {
        Ok(NamedReference::from(self))
    }
}

impl<'a, Id> From<DocumentId> for NamedReference<'a, Id> {
    fn from(id: DocumentId) -> Self {
        Self::Id(id)
    }
}

impl<'a, Id> Nameable<'a, Id> for DocumentId {
    fn name(self) -> Result<NamedReference<'a, Id>, crate::Error> {
        Ok(NamedReference::from(self))
    }
}

impl<'a> Nameable<'a, Self> for u64 {
    fn name(self) -> Result<NamedReference<'a, Self>, crate::Error> {
        Ok(NamedReference::Key(self))
    }
}

impl<'a, Id> NamedReference<'a, Id>
where
    Id: for<'k> Key<'k>,
{
    /// Converts this reference to an owned reference with a `'static` lifetime.
    pub fn into_owned(self) -> NamedReference<'static, Id> {
        match self {
            Self::Name(name) => NamedReference::Name(match name {
                Cow::Owned(string) => Cow::Owned(string),
                Cow::Borrowed(borrowed) => Cow::Owned(borrowed.to_owned()),
            }),
            Self::Id(id) => NamedReference::Id(id),
            Self::Key(key) => NamedReference::Key(key),
        }
    }

    /// Returns this reference's id. If the reference is a name, the
    /// [`NamedCollection::ByNameView`] is queried for the id.
    pub async fn id<Col: NamedCollection<PrimaryKey = Id>, Cn: Connection>(
        &self,
        connection: &Cn,
    ) -> Result<Option<Col::PrimaryKey>, Error> {
        match self {
            Self::Name(name) => connection
                .view::<Col::ByNameView>()
                .with_key(name.as_ref().to_owned())
                .query()
                .await?
                .into_iter()
                .next()
                .map(|e| e.source.id.deserialize())
                .transpose(),
            Self::Id(id) => Ok(Some(id.deserialize()?)),
            Self::Key(id) => Ok(Some(id.clone())),
        }
    }
}

/// A future that resolves to an entry in a [`NamedCollection`].
#[must_use]
pub struct Entry<'a, 'name, Connection, Col, EI, EU>
where
    Col: NamedCollection + SerializedCollection,
    EI: EntryInsert<Col>,
    EU: EntryUpdate<Col>,
{
    state: EntryState<'a, 'name, Connection, Col, EI, EU>,
}

struct EntryBuilder<
    'a,
    'name,
    Connection,
    Col,
    EI: EntryInsert<Col> + 'a,
    EU: EntryUpdate<Col> + 'a,
> where
    Col: SerializedCollection,
{
    name: NamedReference<'name, Col::PrimaryKey>,
    connection: &'a Connection,
    insert: Option<EI>,
    update: Option<EU>,
    retry_limit: usize,
    _collection: PhantomData<Col>,
}

impl<'a, 'name, Connection, Col, EI, EU> Entry<'a, 'name, Connection, Col, EI, EU>
where
    Col: NamedCollection + SerializedCollection + 'static + Unpin,
    Connection: crate::connection::Connection,
    EI: EntryInsert<Col> + 'a + Unpin,
    EU: EntryUpdate<Col> + 'a + Unpin,
    'name: 'a,
{
    async fn execute(
        name: NamedReference<'name, Col::PrimaryKey>,
        connection: &'a Connection,
        insert: Option<EI>,
        update: Option<EU>,
        mut retry_limit: usize,
    ) -> Result<Option<CollectionDocument<Col>>, Error> {
        if let Some(mut existing) = Col::load(name, connection).await? {
            if let Some(update) = update {
                loop {
                    update.call(&mut existing.contents);
                    match existing.update(connection).await {
                        Ok(()) => return Ok(Some(existing)),
                        Err(Error::DocumentConflict(collection, header)) => {
                            // Another client has updated the document underneath us.
                            if retry_limit > 0 {
                                retry_limit -= 1;
                                existing = match Col::load(header.id, connection).await? {
                                    Some(doc) => doc,
                                    // Another client deleted the document before we could reload it.
                                    None => break Ok(None),
                                }
                            } else {
                                break Err(Error::DocumentConflict(collection, header));
                            }
                        }
                        Err(other) => break Err(other),
                    }
                }
            } else {
                Ok(Some(existing))
            }
        } else if let Some(insert) = insert {
            let new_document = insert.call();
            Ok(Some(Col::push(new_document, connection).await?))
        } else {
            Ok(None)
        }
    }
    fn pending(&mut self) -> &mut EntryBuilder<'a, 'name, Connection, Col, EI, EU> {
        match &mut self.state {
            EntryState::Pending(pending) => pending.as_mut().unwrap(),
            EntryState::Executing(_) => unreachable!(),
        }
    }

    /// If an entry with the key doesn't exist, `cb` will be executed to provide
    /// an initial document. This document will be saved before being returned.
    pub fn or_insert_with<F: EntryInsert<Col> + 'a + Unpin>(
        self,
        cb: F,
    ) -> Entry<'a, 'name, Connection, Col, F, EU> {
        Entry {
            state: match self.state {
                EntryState::Pending(Some(EntryBuilder {
                    name,
                    connection,
                    update,
                    retry_limit,
                    ..
                })) => EntryState::Pending(Some(EntryBuilder {
                    name,
                    connection,
                    insert: Some(cb),
                    update,
                    retry_limit,
                    _collection: PhantomData,
                })),
                _ => {
                    unreachable!("attempting to modify an already executing future")
                }
            },
        }
    }

    /// If an entry with the keys exists, `cb` will be executed with the stored
    /// value, allowing an opportunity to update the value. This new value will
    /// be saved to the database before returning. If an error occurs during
    /// update, `cb` may be invoked multiple times, up to the
    /// [`retry_limit`](Self::retry_limit()).
    pub fn update_with<F: EntryUpdate<Col> + 'a + Unpin>(
        self,
        cb: F,
    ) -> Entry<'a, 'name, Connection, Col, EI, F> {
        Entry {
            state: match self.state {
                EntryState::Pending(Some(EntryBuilder {
                    name,
                    connection,
                    insert,
                    retry_limit,
                    ..
                })) => EntryState::Pending(Some(EntryBuilder {
                    name,
                    connection,
                    insert,
                    update: Some(cb),
                    retry_limit,
                    _collection: PhantomData,
                })),
                _ => {
                    unreachable!("attempting to modify an already executing future")
                }
            },
        }
    }

    /// The number of attempts to attempt updating the document using
    /// `update_with` before returning an error.
    pub fn retry_limit(mut self, attempts: usize) -> Self {
        self.pending().retry_limit = attempts;
        self
    }
}

pub trait EntryInsert<Col: SerializedCollection>: Send + Unpin {
    fn call(self) -> Col::Contents;
}

impl<F, Col> EntryInsert<Col> for F
where
    F: FnOnce() -> Col::Contents + Send + Unpin,
    Col: SerializedCollection,
{
    fn call(self) -> Col::Contents {
        self()
    }
}

impl<Col> EntryInsert<Col> for ()
where
    Col: SerializedCollection,
{
    fn call(self) -> Col::Contents {
        unreachable!()
    }
}

pub trait EntryUpdate<Col>: Send + Unpin
where
    Col: SerializedCollection,
{
    fn call(&self, doc: &mut Col::Contents);
}

impl<F, Col> EntryUpdate<Col> for F
where
    F: Fn(&mut Col::Contents) + Send + Unpin,
    Col: NamedCollection + SerializedCollection,
{
    fn call(&self, doc: &mut Col::Contents) {
        self(doc);
    }
}

impl<Col> EntryUpdate<Col> for ()
where
    Col: SerializedCollection,
{
    fn call(&self, _doc: &mut Col::Contents) {
        unreachable!();
    }
}

impl<'a, 'name, Conn, Col, EI, EU> Future for Entry<'a, 'name, Conn, Col, EI, EU>
where
    Col: NamedCollection + SerializedCollection + 'static,
    <Col as Collection>::PrimaryKey: Unpin,
    Conn: Connection,
    EI: EntryInsert<Col> + 'a,
    EU: EntryUpdate<Col> + 'a,
    'name: 'a,
{
    type Output = Result<Option<CollectionDocument<Col>>, Error>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if let Some(EntryBuilder {
            name,
            connection,
            insert,
            update,
            retry_limit,
            ..
        }) = match &mut self.state {
            EntryState::Executing(_) => None,
            EntryState::Pending(builder) => builder.take(),
        } {
            let future = Self::execute(name, connection, insert, update, retry_limit).boxed();
            self.state = EntryState::Executing(future);
        }

        if let EntryState::Executing(future) = &mut self.state {
            future.as_mut().poll(cx)
        } else {
            unreachable!()
        }
    }
}

enum EntryState<'a, 'name, Connection, Col, EI, EU>
where
    Col: NamedCollection + SerializedCollection,
    EI: EntryInsert<Col>,
    EU: EntryUpdate<Col>,
{
    Pending(Option<EntryBuilder<'a, 'name, Connection, Col, EI, EU>>),
    Executing(BoxFuture<'a, Result<Option<CollectionDocument<Col>>, Error>>),
}

/// Executes [`Connection::list()`] when awaited. Also offers methods to
/// customize the options for the operation.
#[must_use]
pub struct List<'a, Cn, Cl>(connection::List<'a, Cn, Cl>)
where
    Cl: Collection;

impl<'a, Cn, Cl> List<'a, Cn, Cl>
where
    Cl: Collection,
{
    /// Lists documents by id in ascending order.
    pub fn ascending(mut self) -> Self {
        self.0 = self.0.ascending();
        self
    }

    /// Lists documents by id in descending order.
    pub fn descending(mut self) -> Self {
        self.0 = self.0.descending();
        self
    }

    /// Sets the maximum number of results to return.
    pub fn limit(mut self, maximum_results: usize) -> Self {
        self.0 = self.0.limit(maximum_results);
        self
    }
}

impl<'a, Cn, Cl> Future for List<'a, Cn, Cl>
where
    Cl: SerializedCollection + Unpin,
    Cl::PrimaryKey: Unpin,
    Cn: Connection,
{
    type Output = Result<Vec<CollectionDocument<Cl>>, Error>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let result = ready!(self.0.poll_unpin(cx));
        Poll::Ready(result.and_then(|docs| docs.collection_documents()))
    }
}

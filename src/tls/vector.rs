use crate::tls::{DefaultSer, Deserializer, Serializer, Sizer};
use std::{
    convert::{TryFrom, TryInto},
    io::{Read, Write},
    mem::size_of,
};
use tls_codec::{Deserialize, Serialize};

pub struct Vector<I = u32, S = DefaultSer>(I, S);

pub type DefVec<I> = Vector<I>;

impl<I, S> Vector<I, S>
where
    I: TryFrom<usize> + TryInto<usize> + Serialize + Deserialize,
{
    pub fn tls_serialized_len<T>(v: &[T]) -> usize
    where
        S: Sizer<T>,
    {
        size_of::<I>() + v.iter().map(|x| S::serialized_len(x)).sum::<usize>()
    }

    pub fn tls_serialize<W, T>(v: &[T], writer: &mut W) -> Result<usize, tls_codec::Error>
    where
        S: Sizer<T> + Serializer<T>,
        W: Write,
    {
        let len = Self::tls_serialized_len(v) - size_of::<I>();
        let len = I::try_from(len).map_err(|_| tls_codec::Error::InvalidVectorLength)?;
        v.iter().try_fold(len.tls_serialize(writer)?, |acc, x| {
            Ok(acc + S::serialize(x, writer)?)
        })
    }

    pub fn tls_deserialize<T, R>(reader: &mut R) -> Result<Vec<T>, tls_codec::Error>
    where
        S: Sizer<T> + Deserializer<T>,
        R: Read,
    {
        let len = I::tls_deserialize(reader)?;
        let len: usize = len
            .try_into()
            .map_err(|_| tls_codec::Error::InvalidVectorLength)?;
        let mut read_len = 0;
        let mut items = Vec::with_capacity(len);
        while read_len < len {
            let item = S::deserialize(reader)?;
            read_len += S::serialized_len(&item);
            items.push(item);
        }
        Ok(items)
    }
}

impl<I, S, T> Sizer<[T]> for Vector<I, S>
where
    I: TryFrom<usize> + TryInto<usize> + Serialize + Deserialize,
    S: Sizer<T>,
{
    fn serialized_len(x: &[T]) -> usize {
        Self::tls_serialized_len(x)
    }
}

impl<I, S, T> Sizer<Vec<T>> for Vector<I, S>
where
    I: TryFrom<usize> + TryInto<usize> + Serialize + Deserialize,
    S: Sizer<T>,
{
    fn serialized_len(x: &Vec<T>) -> usize {
        Self::tls_serialized_len(x)
    }
}

impl<I, S, T> Serializer<[T]> for Vector<I, S>
where
    I: TryFrom<usize> + TryInto<usize> + Serialize + Deserialize,
    S: Sizer<T> + Serializer<T>,
{
    fn serialize<W: Write>(x: &[T], writer: &mut W) -> Result<usize, tls_codec::Error> {
        Self::tls_serialize(x, writer)
    }
}

impl<I, S, T> Serializer<Vec<T>> for Vector<I, S>
where
    I: TryFrom<usize> + TryInto<usize> + Serialize + Deserialize,
    S: Sizer<T> + Serializer<T>,
{
    fn serialize<W: Write>(x: &Vec<T>, writer: &mut W) -> Result<usize, tls_codec::Error> {
        Self::tls_serialize(x, writer)
    }
}

impl<I, S, T> Deserializer<Vec<T>> for Vector<I, S>
where
    I: TryFrom<usize> + TryInto<usize> + Serialize + Deserialize,
    S: Sizer<T> + Deserializer<T>,
{
    fn deserialize<R: Read>(reader: &mut R) -> Result<Vec<T>, tls_codec::Error> {
        Self::tls_deserialize(reader)
    }
}

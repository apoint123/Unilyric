use binrw::{
    BinRead, BinResult, BinWrite, Endian,
    io::{Read, Seek, Write},
};
use serde::{Deserialize, Serialize};

/// 一个自定义的、以 null 字节结尾的 UTF-8 字符串类型。
#[derive(Clone, Eq, PartialEq, Default, Debug, Serialize, Deserialize)]
pub struct NullString(pub String);

impl AsRef<str> for NullString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl std::ops::Deref for NullString {
    type Target = String;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl From<&str> for NullString {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}
impl From<String> for NullString {
    fn from(s: String) -> Self {
        Self(s)
    }
}
impl From<NullString> for String {
    fn from(value: NullString) -> Self {
        value.0
    }
}

impl BinRead for NullString {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        _endian: Endian,
        _args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let mut bytes = Vec::new();
        loop {
            let byte: u8 = BinRead::read_options(reader, Endian::Little, ())?;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
        }

        let cow = String::from_utf8_lossy(&bytes);

        // 如果 cow 被分配了新的内存（意味着有替换发生），就记录一条警告。
        if let std::borrow::Cow::Owned(_) = cow {
            tracing::warn!("[Protocol] 在解析NullString时遇到无效的UTF-8字节序列。");
        }

        Ok(NullString(cow.into_owned()))
    }
}

impl BinWrite for NullString {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        _endian: Endian,
        _args: Self::Args<'_>,
    ) -> BinResult<()> {
        writer.write_all(self.0.as_bytes())?;
        writer.write_all(&[0u8])?;
        Ok(())
    }
}

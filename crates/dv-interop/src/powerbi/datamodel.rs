//! Writer for the `DataModel` part of a `.pbix`.
//!
//! A `.pbix` stores its model as an Analysis Services backup (ABF) compressed
//! with XPress9: a 4 KiB header page, the stored files, a virtual directory
//! and a backup log. The catalog and the column files are produced by
//! [`super::catalog`] and [`super::vertipaq`]; this module wraps them.
//!
//! The layout was reverse engineered from a file saved by Power BI Desktop.
//! Files written this way load in Power BI Desktop and in the engine it
//! ships, and queries on them return the data of the CSV files.

use super::{Model, guid};
use crate::InteropError;

const PAGE: usize = 4096;
const SIGNATURE: &str = "STREAM_STORAGE_SIGNATURE_)!@#$%^&*(";
const COMPRESSED: &str = "This backup was created using XPress9 compression.";

fn err(what: &str, e: impl std::fmt::Display) -> InteropError {
    InteropError::Format(format!("data model: {what}: {e}"))
}

// ---------------------------------------------------------------- ABF

fn utf16_bom(text: &str) -> Vec<u8> {
    let mut out = vec![0xff, 0xfe];
    out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    out
}

fn storage_key(seed: &str) -> String {
    guid(seed).replace('-', "").to_uppercase()[..20].to_string()
}

fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const DB_XML_NAMESPACES: &str = "xmlns=\"http://schemas.microsoft.com/analysisservices/2003/engine\" xmlns:ddl2=\"http://schemas.microsoft.com/analysisservices/2003/engine/2\" xmlns:ddl2_2=\"http://schemas.microsoft.com/analysisservices/2003/engine/2/2\" xmlns:ddl100=\"http://schemas.microsoft.com/analysisservices/2008/engine/100\" xmlns:ddl100_100=\"http://schemas.microsoft.com/analysisservices/2008/engine/100/100\" xmlns:ddl200=\"http://schemas.microsoft.com/analysisservices/2010/engine/200\" xmlns:ddl200_200=\"http://schemas.microsoft.com/analysisservices/2010/engine/200/200\" xmlns:ddl300=\"http://schemas.microsoft.com/analysisservices/2011/engine/300\" xmlns:ddl300_300=\"http://schemas.microsoft.com/analysisservices/2011/engine/300/300\" xmlns:ddl400=\"http://schemas.microsoft.com/analysisservices/2012/engine/400\" xmlns:ddl400_400=\"http://schemas.microsoft.com/analysisservices/2012/engine/400/400\" xmlns:ddl410=\"http://schemas.microsoft.com/analysisservices/2012/engine/410\" xmlns:ddl410_410=\"http://schemas.microsoft.com/analysisservices/2012/engine/410/410\" xmlns:ddl500=\"http://schemas.microsoft.com/analysisservices/2013/engine/500\" xmlns:ddl500_500=\"http://schemas.microsoft.com/analysisservices/2013/engine/500/500\" xmlns:ddl600=\"http://schemas.microsoft.com/analysisservices/2013/engine/600\" xmlns:ddl600_600=\"http://schemas.microsoft.com/analysisservices/2013/engine/600/600\" xmlns:ddl700=\"http://schemas.microsoft.com/analysisservices/2018/engine/700\" xmlns:ddl700_700=\"http://schemas.microsoft.com/analysisservices/2018/engine/700/700\" xmlns:ddl800=\"http://schemas.microsoft.com/analysisservices/2018/engine/800\" xmlns:ddl800_800=\"http://schemas.microsoft.com/analysisservices/2018/engine/800/800\" xmlns:ddl900=\"http://schemas.microsoft.com/analysisservices/2019/engine/900\" xmlns:ddl900_900=\"http://schemas.microsoft.com/analysisservices/2019/engine/900/900\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"";

fn database_xml(id: &str, object_id: &str, unique_id: &str) -> String {
    format!(
        "<Load {DB_XML_NAMESPACES}><ObjectDefinition><Database><Name>{id}</Name><ID>{id}</ID><CreatedTimestamp>2026-01-01T00:00:00.000000</CreatedTimestamp><LastSchemaUpdate>2026-01-01T00:00:00.000000</LastSchemaUpdate><ObjectVersion>0</ObjectVersion><ObjectID>{object_id}</ObjectID><Ordinal>1</Ordinal><PersistLocation>0</PersistLocation><System>false</System><DataFileList>0.CryptKey.bin</DataFileList><Description/><ddl800:DbUniqueId>{unique_id}</ddl800:DbUniqueId><AggregationPrefix/><Language>1033</Language><Collation/><ddl400:DefaultCollationVersion>100</ddl400:DefaultCollationVersion><Visible>true</Visible><MasterDataSourceID/><ProcessingPriority>0</ProcessingPriority><ddl200_200:StorageEngineUsed valuens=\"ddl500_500\">TabularMetadata</ddl200_200:StorageEngineUsed><ddl200:CompatibilityLevel>1550</ddl200:CompatibilityLevel><Translations/><DataSourceImpersonationInfo><ImpersonationMode>Default</ImpersonationMode><Account/><Password>{IMPERSONATION_PASSWORD}</Password></DataSourceImpersonationInfo><DataVersion>0</DataVersion></Database></ObjectDefinition></Load>"
    )
}

struct Stored {
    /// Name in the virtual directory.
    key: String,
    bytes: Vec<u8>,
}

/// Builds the decoded ABF stream.
fn abf(name: &str, catalog: Vec<u8>, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let db_id = guid(&format!("db:{name}"));
    let object_id = guid(&format!("object:{name}")).to_uppercase();
    let unique_id = guid(&format!("unique:{name}")).to_uppercase();
    let root = format!(
        "\\\\?\\C:\\Users\\davilize\\AppData\\Local\\Microsoft\\Power BI Desktop\\AnalysisServicesWorkspaces\\AnalysisServicesWorkspace_{}\\Data",
        guid(&format!("workspace:{name}"))
    );
    let db_path = format!("{root}\\{db_id}.0.db");

    let db_xml = Stored {
        key: storage_key("dbxml"),
        bytes: database_xml(&db_id, &object_id, &unique_id).into_bytes(),
    };
    let crypt = Stored {
        key: storage_key("crypt"),
        bytes: crypt_key(),
    };
    let meta = Stored {
        key: storage_key("meta"),
        bytes: catalog,
    };
    let additional = utf16_bom("<Property><ProductName>Default</ProductName></Property>");
    let partitions = utf16_bom("<Partitions/>");

    let timestamp = "134399582248779066";
    let entry = |path: &str, offset: usize, size: usize| {
        format!(
            "<BackupFile><Path>{path}</Path><Size>{size}</Size><m_cbOffsetHeader>{offset}</m_cbOffsetHeader><Delete>false</Delete><CreatedTimestamp>{timestamp}</CreatedTimestamp><Access>{timestamp}</Access><LastWriteTime>{timestamp}</LastWriteTime></BackupFile>"
        )
    };

    // The stored files, back to back after the header page.
    let mut body: Vec<u8> = Vec::new();
    let mut directory = String::from("<VirtualDirectory>");
    let base = PAGE;
    let put = |path: &str, bytes: &[u8], body: &mut Vec<u8>, directory: &mut String| {
        directory.push_str(&entry(path, base + body.len(), bytes.len()));
        body.extend_from_slice(bytes);
    };
    put("ADDITIONAL_LOG", &additional, &mut body, &mut directory);
    put("PARTITIONS", &partitions, &mut body, &mut directory);
    for stored in [&db_xml, &crypt, &meta] {
        put(&stored.key, &stored.bytes, &mut body, &mut directory);
    }
    let extra: Vec<(String, Stored)> = files
        .iter()
        .map(|(path, bytes)| {
            (
                path.clone(),
                Stored {
                    key: storage_key(path),
                    bytes: bytes.clone(),
                },
            )
        })
        .collect();
    for (_, stored) in &extra {
        put(&stored.key, &stored.bytes, &mut body, &mut directory);
    }

    let file = |path: &str, stored: &Stored| {
        format!(
            "<BackupFile><Path>{}</Path><StoragePath>{}</StoragePath><LastWriteTime>134399581605534554</LastWriteTime><Size>{}</Size></BackupFile>",
            xml_text(path),
            stored.key,
            stored.bytes.len()
        )
    };
    let group = |class: &str, version: &str, location: &str, object: &str, files: String| {
        format!(
            "<FileGroup><Class>{class}</Class><ID>{db_id}</ID><Name>{db_id}</Name><ObjectVersion>{version}</ObjectVersion><PersistLocation>{location}</PersistLocation><PersistLocationPath>{}</PersistLocationPath><StorageLocationPath/><ObjectID>{object}</ObjectID><FileList>{files}</FileList></FileGroup>",
            xml_text(&db_path)
        )
    };
    let log = format!(
        "<BackupLog><BackupRestoreSyncVersion>11.53</BackupRestoreSyncVersion><ServerRoot>{}</ServerRoot><SvrEncryptPwdFlag>true</SvrEncryptPwdFlag><ServerEnableBinaryXML>false</ServerEnableBinaryXML><ServerEnableCompression>false</ServerEnableCompression><CompressionFlag>false</CompressionFlag><EncryptionFlag>false</EncryptionFlag><ObjectName>{db_id}</ObjectName><ObjectId>{db_id}</ObjectId><Write>ReadWrite</Write><OlapInfo>false</OlapInfo><IsTabular>true</IsTabular><Collations/><Languages><Language>1033</Language></Languages><FileGroups>{}{}</FileGroups></BackupLog>",
        xml_text(&root),
        group(
            "100002",
            "0",
            "0",
            &object_id,
            file(&format!("{root}\\{db_id}.0.db.xml"), &db_xml)
                + &file(&format!("{db_path}\\0.CryptKey.bin"), &crypt)
        ),
        group(
            "100069",
            "-1",
            "-1",
            "00000000-0000-0000-0000-000000000000",
            file(&format!("{db_path}\\metadata.sqlitedb"), &meta)
                + &extra
                    .iter()
                    .map(|(path, stored)| file(&format!("{db_path}\\{path}"), stored))
                    .collect::<String>()
        ),
    );
    let log_bytes = utf16_bom(&log);
    put("LOG", &log_bytes, &mut body, &mut directory);
    directory.push_str("</VirtualDirectory>");

    let directory_offset = base + body.len();
    let files = 5 + 1 + extra.len();
    let mut stream = Vec::new();
    // Header page: signature, then the log header, NUL padded to 4 KiB.
    stream.extend(utf16_bom(SIGNATURE));
    let header = format!(
        "<BackupLog><BackupRestoreSyncVersion>140</BackupRestoreSyncVersion><Fault>false</Fault><faultcode>0</faultcode><ErrorCode>false</ErrorCode><EncryptionFlag>false</EncryptionFlag><EncryptionKey>0</EncryptionKey><ApplyCompression>false</ApplyCompression><m_cbOffsetHeader>{directory_offset}</m_cbOffsetHeader><DataSize>{}</DataSize><Files>{files}</Files><ObjectID>{}</ObjectID><m_cbOffsetData>{PAGE}</m_cbOffsetData></BackupLog>",
        directory.len(),
        guid(&format!("backup:{name}")).to_uppercase(),
    );
    stream.extend(header.encode_utf16().flat_map(u16::to_le_bytes));
    stream.resize(PAGE, 0);
    stream.extend(body);
    stream.extend(directory.into_bytes());
    let padded = stream.len().div_ceil(PAGE) * PAGE;
    stream.resize(padded, 0);
    stream
}

/// The 144-byte `0.CryptKey.bin` of a model saved by Power BI Desktop.
///
/// Desktop refuses a model whose key does not validate ("Invalid cryptographic
/// key"), and the key cannot be generated here, so every written model carries
/// this one, together with the matching encrypted impersonation password in the
/// database definition. It only protects data-source credentials, and none are
/// stored in the models written here.
const CRYPT_KEY: [u8; 144] = [
    0x98, 0xbc, 0x21, 0x5d, 0x2d, 0x8d, 0xe6, 0x4e, 0xa8, 0xe5, 0xd0, 0x38, 0xaa, 0xc9, 0x44, 0x41,
    0x04, 0x00, 0x00, 0x00, 0x30, 0x00, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x02, 0x00, 0x00, 0x03, 0x66, 0x00, 0x00, 0x00, 0xa4, 0x00, 0x00, 0x51, 0xb9, 0x92, 0x85,
    0xe0, 0x13, 0x0d, 0x2f, 0x4c, 0xf4, 0xad, 0xba, 0x2c, 0xb0, 0xb6, 0x97, 0x2c, 0x20, 0x46, 0x75,
    0xf7, 0x98, 0x0d, 0x19, 0x00, 0xd0, 0xb5, 0xaa, 0x11, 0xfe, 0xaf, 0x64, 0xa8, 0x94, 0x35, 0xaf,
    0x9e, 0xa5, 0x22, 0x64, 0xfd, 0x8c, 0x0c, 0x3a, 0x0d, 0xee, 0x39, 0x79, 0x6d, 0xae, 0x4e, 0xeb,
    0xb5, 0x66, 0x1f, 0xfd, 0x64, 0x60, 0xc4, 0x75, 0x96, 0x2c, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x98, 0xbc, 0x21, 0x5d, 0x2d, 0x8d, 0xe6, 0x4e, 0xa8, 0xe5, 0xd0, 0x38, 0xaa, 0xc9, 0x44, 0x41,
];

const IMPERSONATION_PASSWORD: &str = "CAAAAAAAAABiGyEcTegjxH4bg2p+xv/r 
";

fn crypt_key() -> Vec<u8> {
    CRYPT_KEY.to_vec()
}

/// Compresses the stream like Desktop: the XPress9 header, then chunks of
/// `(uncompressed size, compressed size, bytes)`.
fn compress(stream: &[u8]) -> Result<Vec<u8>, InteropError> {
    let mut out = Vec::new();
    out.extend(COMPRESSED.encode_utf16().flat_map(u16::to_le_bytes));
    out.resize(102, 0);
    let mut encoder = ripbi_xpress9::Encoder::new().map_err(|e| err("compression", e))?;
    for chunk in stream.chunks(8 * 1024 * 1024) {
        let packed = encoder
            .encode_chunk(chunk)
            .map_err(|e| err("compression", e))?;
        out.extend((chunk.len() as u32).to_le_bytes());
        out.extend((packed.len() as u32).to_le_bytes());
        out.extend(packed);
    }
    Ok(out)
}

/// The compressed `DataModel` part for `model`.
pub(crate) fn data_model(
    model: &Model,
    queries: &[(String, Vec<String>)],
    name: &str,
    values: &std::collections::HashMap<(String, String), Vec<String>>,
) -> Result<Vec<u8>, InteropError> {
    let built = super::catalog::build(model, queries, values)?;
    compress(&abf(name, built.catalog, &built.files))
}

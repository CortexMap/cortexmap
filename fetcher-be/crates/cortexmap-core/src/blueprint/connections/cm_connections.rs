#[derive(Debug, Clone)]
pub struct Connections {
    pub db: Database,
    pub s3_info: S3Info,
}

impl Connections {
    pub fn db_url(&self) -> &str {
        match &self.db {
            Database::Postgresql(pg) => &pg.url,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Database {
    Postgresql(Postgresql),
}

#[derive(Debug, Clone)]
pub struct Postgresql {
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct S3Info {
    pub endpoint: Option<String>,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub bucket: String,
}

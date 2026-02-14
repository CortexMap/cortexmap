#[derive(Debug, Clone)]
pub struct Connections {
    pub db: Database,
    pub s3_info: S3Info,
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
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
}

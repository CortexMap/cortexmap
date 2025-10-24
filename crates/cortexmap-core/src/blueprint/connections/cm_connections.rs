pub struct Connections {
    pub db: Database,
    pub s3_info: S3Info,
}

pub enum Database {
    Postgresql(Postgresql),
}

pub struct Postgresql {
    pub url: String,
}

pub struct S3Info {
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
}

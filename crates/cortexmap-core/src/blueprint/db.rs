pub enum Database {
    Postgresql(Postgresql),
}

pub struct Postgresql {
    pub url: String,
}

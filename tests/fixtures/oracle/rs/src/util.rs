use crate::net::client::connect;

pub(crate) fn helper() -> u8 {
    connect();
    7
}

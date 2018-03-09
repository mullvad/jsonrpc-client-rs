#[allow(dead_code)]
pub mod test_server;

#[allow(unused_macros)]
macro_rules! assert_err {
    ($error_chain1:expr, $error_chain2:expr) => {
        for (error1, error2) in $error_chain1.iter().zip($error_chain2.iter()) {
            assert_eq!(format!("{}", error1), format!("{}", error2));
        }
    }
}

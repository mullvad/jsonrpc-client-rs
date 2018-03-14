#[allow(dead_code)]
pub mod test_server;

#[allow(unused_macros)]
macro_rules! assert_error_chain_message {
    (@ready $error_chain1:expr, $error_chain2:expr) => {
        for (error1, error2) in $error_chain1.iter().zip($error_chain2.iter()) {
            assert_eq!(format!("{}", error1), format!("{}", error2));
        }
    };

    (
        $result:expr,
        $error_package:ident :: $error_kind:ident
        $( -> $error_packages:ident :: $error_kinds:ident )*
    ) => {
        assert!($result.is_err(), "expected an error, but got Ok(_)");

        let actual_error = $result.unwrap_err();
        let root_cause = $error_package::Error::from($error_package::ErrorKind::$error_kind);

        assert_error_chain_message! {
            @building
            actual_error,
            root_cause,
            -> $error_package::$error_kind $( -> $error_packages::$error_kinds )*
        };
    };

    (@building $error_chain1:expr, $error_chain2:expr,) => {
        assert_error_chain_message!(@ready $error_chain1, $error_chain2);
    };

    (
        @building
        $error_chain1:expr,
        $error_chain2:expr,
        -> $error_package:ident :: $error_kind:ident
        $( -> $error_packages:ident :: $error_kinds:ident )*
    ) => {
        let error = $error_package::Error::with_chain(
            $error_chain2,
            $error_package::ErrorKind::$error_kind,
        );
        assert_error_chain_message! {
            @building
            $error_chain1,
            error,
            $( -> $error_packages::$error_kinds )*
        };
    }
}

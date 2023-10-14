#[macro_export]
macro_rules! method_decl {
    ($name:ident, $arg:ty, $ret:ty) => {
        pub const $name: $crate::handler::MethodDecl<false, $arg, $ret> =
            $crate::handler::MethodDecl::new(concat!(stringify!($name)), $crate::const_uuid_v4!());
    };
}

#[macro_export]
macro_rules! method_decl_owned {
    ($name:ident, $arg:ty, $ret:ty) => {
        pub const $name: $crate::handler::MethodDecl<true, $arg, $ret> =
            $crate::handler::MethodDecl::new(concat!(stringify!($name)), $crate::const_uuid_v4!());
    };
}

#[cfg(feature = "bus_dbg")]
#[macro_export]
macro_rules! handler_decl_t {
    ($desc:literal) => {
        $crate::msg::HandlerType {
            id: $crate::const_uuid_v4!(),
            id_desc: $crate::msg::Str::Borrowed($desc),
        }
    };
}

#[cfg(not(feature = "bus_dbg"))]
#[macro_export]
macro_rules! handler_decl_t {
    ($desc:literal) => {
        $crate::msg::HandlerType {
            id: $crate::const_uuid_v4!(),
        }
    };
}

#[allow(unused_macros)]
macro_rules! method_decl {
    ($name:ident, $arg:ty, $ret:ty) => {
        pub const $name: $crate::bus::handler::MethodDecl<false, $arg, $ret> =
            $crate::bus::handler::MethodDecl::new(concat!(stringify!($name)));
    };
}

#[allow(unused_macros)]
macro_rules! method_decl_owned {
    ($name:ident, $arg:ty, $ret:ty) => {
        pub const $name: $crate::bus::handler::MethodDecl<true, $arg, $ret> =
            $crate::bus::handler::MethodDecl::new(concat!(stringify!($name)));
    };
}

macro_rules! handler_decl_t {
    ($desc:literal) => {
        $crate::bus::msg::HandlerType {
            id: $crate::bus::id::const_uuid_v4(),
            #[cfg(feature = "bus_dbg")]
            id_desc: $crate::bus::msg::Str::Borrowed($desc),
        }
    };
}

pub(crate) use handler_decl_t;
#[allow(unused_imports)]
pub(crate) use method_decl;
#[allow(unused_imports)]
pub(crate) use method_decl_owned;

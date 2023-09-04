use std::fmt::Write;
use std::mem::size_of;

#[doc(hidden)]
pub fn type_name<T>() -> &'static str {
    std::any::type_name::<T>().split(":").last().unwrap()
}

pub fn sfmt(nbytes: usize) -> String {
    const PRECISION: usize = 2;
    let (unit, pow): (_, u32) = match () {
        _ if nbytes >= 10usize.pow(12) => ("TB", 12),
        _ if nbytes >= 10usize.pow(9) => ("GB", 9),
        _ if nbytes >= 10usize.pow(6) => ("MB", 6),
        _ if nbytes >= 10usize.pow(3) => ("KB", 3),
        _ => ("B", 0),
    };
    let nbytes_in_unit = nbytes as f64 / 10usize.pow(pow) as f64;
    format!("{:.*}{unit}", PRECISION, nbytes_in_unit)
}

pub fn print_inf<T: Info>() {
    const TAB: &str = "  ";

    let fields = <T as Info>::info2().expect("print_inf called on non-info implementing type");
    let size = size_of::<T>();
    let fsize = sfmt(size);
    let name = type_name::<T>();

    let mut out = String::new();
    writeln!(out, "{TAB}{name} = {fsize}").unwrap();
    fn inner(
        nidt: usize,
        prev_typ: fn() -> Option<Vec<Field>>,
        out: &mut String,
        fields: &[Field],
    ) {
        let idt = TAB.repeat(nidt);
        let len = fields.len();
        for (i, field) in fields.into_iter().enumerate() {
            let ptr = if field.is_pointer { " ->" } else { ":" };
            let bar = if i == len - 1 { "\\" } else { "|" };
            match field.kind {
                FieldKind::Single {
                    ref name,
                    typ,
                    size,
                } => {
                    let fsize = sfmt(size);
                    let infstat = if (field.info_impl)().is_none() {
                        "[noinf]".to_string()
                    } else {
                        format!("[inf {typ}]")
                    };
                    writeln!(out, "{idt}{bar} {name}{ptr} {typ} = {fsize} {infstat}").unwrap();
                }
                FieldKind::Array {
                    ref name,
                    elem_t,
                    elem_size,
                    ref len_name,
                    len,
                    total_size,
                } => {
                    let felem = sfmt(elem_size);
                    let ftotal = sfmt(total_size);
                    let infstat = if (field.info_impl)().is_none() {
                        "[noinf]".to_string()
                    } else {
                        format!("[inf {elem_t}]")
                    };
                    writeln!(
                        out,
                            "{idt}{bar} {name}{ptr} [{elem_t}={felem} X {len_name}={len}] = {ftotal} {infstat}"
                    ).unwrap();
                }
                FieldKind::ChunkedLinkedList {
                    ref name,
                    metadata_size,
                    elem_t,
                    elem_size,
                    ref chunk_len_name,
                    chunk_len,
                    total_size,
                } => {
                    let felem = sfmt(elem_size);
                    let fmeta = sfmt(metadata_size);
                    let ftotal = sfmt(total_size);
                    let infstat = if (field.info_impl)().is_none() {
                        "[noinf]".to_string()
                    } else {
                        format!("[inf {elem_t}]")
                    };
                    writeln!(
                        out,
                        "{idt}{bar} {name}{ptr} [[{elem_t}={felem} X {chunk_len_name}={chunk_len}]] (+{fmeta}) = {ftotal} {infstat}"
                    ).unwrap();
                }
                FieldKind::PointerUnion(ref fields) => {
                    if !field.is_pointer {
                        warn!("pointer union with non-pointer field??");
                    }
                    writeln!(out, "{idt}{bar} {{ pointer union [{}] }}", fields.len()).unwrap();
                    inner(nidt + 1, field.info_impl, out, fields);
                    continue; // do not call the info_impl of self (infinite recursion), go on to the next field
                }
            }
            if prev_typ != field.info_impl {
                if let Some(fields) = (field.info_impl)() {
                    inner(nidt + 1, field.info_impl, out, &fields)
                }
            }
        }
    }
    inner(1, <T as Info>::info2, &mut out, &fields);

    info!("\n{}", out.trim());
}

pub trait Info {
    fn info2() -> Option<Vec<Field>>;
}

impl<T> Info for T {
    default fn info2() -> Option<Vec<Field>> {
        None
    }
}

pub struct Field {
    pub is_pointer: bool,
    pub kind: FieldKind,
    pub info_impl: fn() -> Option<Vec<Field>>,
}

pub enum FieldKind {
    Single {
        name: String,
        typ: &'static str,
        size: usize,
    },
    Array {
        name: String,
        elem_t: &'static str,
        elem_size: usize,
        len_name: String, // name of constant defining the length
        len: usize,
        total_size: usize,
    },
    ChunkedLinkedList {
        name: String,
        metadata_size: usize,
        elem_t: &'static str,
        elem_size: usize,
        chunk_len_name: String,
        chunk_len: usize,
        total_size: usize,
    },
    // union, where each element is a pointer to something
    PointerUnion(Vec<Field>),
}

impl Info for super::DataGroup {
    fn info2() -> Option<Vec<Field>> {
        Some(vec![Field {
            is_pointer: true,
            kind: FieldKind::PointerUnion(vec![
                Field {
                    is_pointer: true,
                    kind: FieldKind::Single {
                        name: "periodic".into(),
                        typ: type_name::<super::DataGroupPeriodic>(),
                        size: size_of::<super::DataGroupPeriodic>(),
                    },
                    info_impl: super::DataGroupPeriodic::info2,
                },
                Field {
                    is_pointer: true,
                    kind: FieldKind::Single {
                        name: "sporadic".into(),
                        typ: type_name::<super::DataGroupSporadic>(),
                        size: size_of::<super::DataGroupSporadic>(),
                    },
                    info_impl: super::DataGroupSporadic::info2,
                },
            ]),
            info_impl: Self::info2,
        }])
    }
}

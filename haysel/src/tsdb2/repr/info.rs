use std::fmt::Write;
use std::mem::size_of;

use crate::tsdb2::{
    alloc::{
        ptr::{Ptr, Void},
        util::ChunkedLinkedList,
    },
    tuning,
};

fn type_name<T>() -> &'static str {
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

    let fields = <T as Info>::info2();
    let size = size_of::<T>();
    let fsize = sfmt(size);
    let name = type_name::<T>();

    let mut out = String::new();
    writeln!(out, "{TAB}{name} = {fsize}").unwrap();
    fn inner(nidt: usize, out: &mut String, fields: &[Field]) {
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
                    let infstat = if field.info_impl.is_none() {
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
                    let infstat = if field.info_impl.is_none() {
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
                    let infstat = if field.info_impl.is_none() {
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
                    if field.info_impl.is_some() {
                        error!("pointer union field with `info_impl` set, this will cause very strange output");
                    }
                    writeln!(out, "{idt}{bar} {{ pointer union [{}] }}", fields.len()).unwrap();
                    inner(nidt + 1, out, fields);
                }
            }
            if let Some(info_impl) = field.info_impl {
                inner(nidt + 1, out, &info_impl())
            }
        }
    }
    inner(1, &mut out, &fields);

    info!("\n{}", out.trim());
}

pub trait Info {
    fn info2() -> Vec<Field> {
        unimplemented!("called default <Info>::info2 function")
    }
}

pub struct Field {
    pub is_pointer: bool,
    pub kind: FieldKind,
    pub info_impl: Option<fn() -> Vec<Field>>,
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

impl Info for super::DBEntrypoint {
    fn info2() -> Vec<Field> {
        vec![Field {
            is_pointer: false,
            kind: FieldKind::Single {
                name: "stations".into(),
                typ: type_name::<super::MapStations>(),
                size: size_of::<super::MapStations>(),
            },
            info_impl: Some(super::MapStations::info2),
        }]
    }
}

impl Info for super::MapStations {
    fn info2() -> Vec<Field> {
        vec![Field {
            is_pointer: true,
            kind: FieldKind::ChunkedLinkedList {
                name: "map".into(),
                metadata_size: size_of::<ChunkedLinkedList<0, ()>>(),
                elem_t: type_name::<super::Station>(),
                elem_size: size_of::<super::Station>(),
                chunk_len_name: "STATION_MAP_CHUNK_SIZE".into(),
                chunk_len: tuning::STATION_MAP_CHUNK_SIZE,
                total_size: size_of::<
                    ChunkedLinkedList<{ tuning::STATION_MAP_CHUNK_SIZE }, super::Station>,
                >(),
            },
            info_impl: Some(super::Station::info2),
        }]
    }
}

impl Info for super::Station {
    fn info2() -> Vec<Field> {
        vec![
            Field {
                is_pointer: false,
                kind: FieldKind::Single {
                    name: "id".into(),
                    typ: type_name::<super::StationID>(),
                    size: size_of::<super::StationID>(),
                },
                info_impl: None,
            },
            Field {
                is_pointer: true,
                kind: FieldKind::ChunkedLinkedList {
                    name: "channels".into(),
                    metadata_size: size_of::<ChunkedLinkedList<0, ()>>(),
                    elem_t: type_name::<super::Channel>(),
                    elem_size: size_of::<super::Channel>(),
                    chunk_len_name: "CHANNEL_MAP_CHUNK_SIZE".into(),
                    chunk_len: tuning::CHANNEL_MAP_CHUNK_SIZE,
                    total_size: size_of::<
                        ChunkedLinkedList<{ tuning::CHANNEL_MAP_CHUNK_SIZE }, super::Channel>,
                    >(),
                },
                info_impl: Some(super::Channel::info2),
            },
        ]
    }
}

impl Info for super::Channel {
    fn info2() -> Vec<Field> {
        vec![
            Field {
                is_pointer: false,
                kind: FieldKind::Single {
                    name: "id".into(),
                    typ: type_name::<super::ChannelID>(),
                    size: size_of::<super::ChannelID>(),
                },
                info_impl: None,
            },
            Field {
                is_pointer: false,
                kind: FieldKind::Single {
                    name: "metadata".into(),
                    typ: type_name::<super::ChannelMetadata>(),
                    size: size_of::<super::ChannelMetadata>(),
                },
                info_impl: Some(super::ChannelMetadata::info2),
            },
            Field {
                is_pointer: false,
                kind: FieldKind::Single {
                    name: "_pad".into(),
                    typ: type_name::<[u8; 7]>(),
                    size: size_of::<[u8; 7]>(),
                },
                info_impl: None,
            },
            Field {
                is_pointer: true,
                kind: FieldKind::ChunkedLinkedList {
                    name: "data".into(),
                    metadata_size: size_of::<ChunkedLinkedList<0, ()>>(),
                    elem_t: type_name::<super::DataGroupIndex>(),
                    elem_size: size_of::<super::DataGroupIndex>(),
                    chunk_len_name: "DATA_IDEX_CHUNK_SIZE".into(),
                    chunk_len: tuning::DATA_INDEX_CHUNK_SIZE,
                    total_size: size_of::<
                        ChunkedLinkedList<{ tuning::DATA_INDEX_CHUNK_SIZE }, super::DataGroupIndex>,
                    >(),
                },
                info_impl: Some(super::DataGroupIndex::info2),
            },
        ]
    }
}

impl Info for super::ChannelMetadata {
    fn info2() -> Vec<Field> {
        vec![Field {
            is_pointer: false,
            kind: FieldKind::Single {
                name: "group_type".into(),
                typ: type_name::<u8>(),
                size: size_of::<u8>(),
            },
            info_impl: None,
        }]
    }
}

impl Info for super::DataGroupIndex {
    fn info2() -> Vec<Field> {
        vec![
            Field {
                is_pointer: false,
                kind: FieldKind::Single {
                    name: "after".into(),
                    typ: type_name::<u64>(),
                    size: size_of::<u64>(),
                },
                info_impl: None,
            },
            Field {
                is_pointer: true,
                kind: FieldKind::Single {
                    name: "group".into(),
                    typ: type_name::<super::DataGroup>(),
                    size: size_of::<super::DataGroup>(),
                },
                // an enum!
                info_impl: Some(super::DataGroup::info2),
            },
        ]
    }
}

impl Info for super::DataGroup {
    fn info2() -> Vec<Field> {
        vec![Field {
            is_pointer: true,
            kind: FieldKind::PointerUnion(vec![
                Field {
                    is_pointer: true,
                    kind: FieldKind::Single {
                        name: "periodic".into(),
                        typ: type_name::<super::DataGroupPeriodic>(),
                        size: size_of::<super::DataGroupPeriodic>(),
                    },
                    info_impl: Some(super::DataGroupPeriodic::info2),
                },
                Field {
                    is_pointer: true,
                    kind: FieldKind::Single {
                        name: "sporadic".into(),
                        typ: type_name::<super::DataGroupSporadic>(),
                        size: size_of::<super::DataGroupSporadic>(),
                    },
                    info_impl: Some(super::DataGroupSporadic::info2),
                },
            ]),
            info_impl: None,
        }]
    }
}

impl Info for super::DataGroupPeriodic {
    fn info2() -> Vec<Field> {
        vec![
            Field {
                is_pointer: false,
                kind: FieldKind::Single {
                    name: "avg_dt".into(),
                    typ: type_name::<u32>(),
                    size: size_of::<u32>(),
                },
                info_impl: None,
            },
            Field {
                is_pointer: false,
                kind: FieldKind::Single {
                    name: "used".into(),
                    typ: type_name::<u16>(),
                    size: size_of::<u16>(),
                },
                info_impl: None,
            },
            Field {
                is_pointer: false,
                kind: FieldKind::Array {
                    name: "dt".into(),
                    elem_t: type_name::<i16>(),
                    elem_size: size_of::<i16>(),
                    len_name: "DATA_GROUP_PERIODIC_SIZE-1".into(),
                    len: tuning::DATA_GROUP_PERIODIC_SIZE - 1,
                    total_size: size_of::<[i16; tuning::DATA_GROUP_PERIODIC_SIZE - 1]>(),
                },
                info_impl: None,
            },
            Field {
                is_pointer: false,
                kind: FieldKind::Array {
                    name: "data".into(),
                    elem_t: type_name::<f32>(),
                    elem_size: size_of::<f32>(),
                    len_name: "DATA_GROUP_PERIODIC_SIZE-1".into(),
                    len: tuning::DATA_GROUP_PERIODIC_SIZE - 1,
                    total_size: size_of::<[f32; tuning::DATA_GROUP_PERIODIC_SIZE - 1]>(),
                },
                info_impl: None,
            },
        ]
    }
}

impl Info for super::DataGroupSporadic {
    fn info2() -> Vec<Field> {
        vec![
            Field {
                is_pointer: false,
                kind: FieldKind::Array {
                    name: "dt".into(),
                    elem_t: type_name::<u32>(),
                    elem_size: size_of::<u32>(),
                    len_name: "DATA_GROUP_SPORAIDC_SIZE".into(),
                    len: tuning::DATA_GROUP_PERIODIC_SIZE,
                    total_size: size_of::<[u32; tuning::DATA_GROUP_PERIODIC_SIZE]>(),
                },
                info_impl: None,
            },
            Field {
                is_pointer: false,
                kind: FieldKind::Array {
                    name: "data".into(),
                    elem_t: type_name::<f32>(),
                    elem_size: size_of::<f32>(),
                    len_name: "DATA_GROUP_SPORADIC_SIZE".into(),
                    len: tuning::DATA_GROUP_SPORADIC_SIZE,
                    total_size: size_of::<[f32; tuning::DATA_GROUP_SPORADIC_SIZE]>(),
                },
                info_impl: None,
            },
        ]
    }
}

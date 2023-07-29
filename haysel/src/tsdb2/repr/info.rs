use std::mem::size_of;

use crate::tsdb2::{
    alloc::ptr::{Ptr, Void},
    tuning,
};

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
    format!("{:.*}{unit}", PRECISION, nbytes_in_unit,)
}

pub trait Info {
    fn info();
}

impl Info for super::DBEntrypoint {
    fn info() {
        info!(
            "DBEntrypoint:\n\tstation map:\t{}",
            sfmt(size_of::<super::MapStations>())
        )
    }
}

impl Info for super::MapStations {
    fn info() {
        info!("MapStations:\n\tpointer:\t{}", sfmt(size_of::<Ptr<Void>>()))
    }
}

impl Info for super::Station {
    fn info() {
        info!(
            "Station:\
        \n\tdata:\t{}\
        \n\tpointer:\t{}\
        \n\ttotal:\t{}",
            sfmt(size_of::<super::StationID>()),
            sfmt(size_of::<Ptr<Void>>()),
            sfmt(size_of::<super::StationID>() + size_of::<Ptr<Void>>())
        );
    }
}

impl Info for super::Channel {
    fn info() {
        let meta_size = size_of::<super::ChannelID>() + size_of::<super::ChannelMetadata>();
        let padding = 7;
        let list_csize = tuning::DATA_INDEX_CHUNK_SIZE;
        let list_head_size = size_of::<
            super::ChunkedLinkedList<{ tuning::DATA_INDEX_CHUNK_SIZE }, super::DataGroupIndex>,
        >();
        let all = meta_size + padding + list_head_size;
        assert_eq!(all, size_of::<super::Channel>());
        let meta_size = sfmt(meta_size);
        let padding = sfmt(padding);
        let list_head_size = sfmt(list_head_size);
        let all = sfmt(all);

        info!(
            "Channel:\
        \n\tmetadata size:\t{meta_size}\
        \n\tpadding:\t{padding}\
        \n\tvar list csize:\t{list_csize}\
        \n\tvar list head:\t{list_head_size}\
        \n\ttotal:\t{all}"
        );
    }
}

impl Info for super::ChannelMetadata {
    fn info() {
        info!(
            "ChannelMetadata:\n\tdata size:\t{}",
            sfmt(size_of::<super::DataGroupType>())
        )
    }
}

impl Info for super::DataGroupIndex {
    fn info() {
        info!(
            "DataGroupIndex:\
            \n\tdata size:\t{}\
            \n\tpointer:\t{}\
            \n\toverall:\t{}",
            sfmt(size_of::<u64>()),
            sfmt(size_of::<Ptr<Void>>()),
            sfmt(size_of::<u64>() + size_of::<Ptr<Void>>()),
        )
    }
}

impl Info for super::DataGroup {
    fn info() {
        info!(
            "DataGroup [union]:\n\tdata size:\t{}",
            sfmt(size_of::<Ptr<Void>>())
        )
    }
}

impl Info for super::DataGroupType {
    fn info() {
        info!(
            "DataGroupType [enum]:\
            \n\tsize:\t{}",
            sfmt(1)
        )
    }
}

impl Info for super::DataGroupPeriodic {
    fn info() {
        let const_cost = 6;
        let var_cost = 6;
        let var_set = tuning::DATA_GROUP_PERIODIC_SIZE - 1;
        let var_all = var_cost * var_set;
        let all = var_all + const_cost;
        assert_eq!(all, size_of::<Self>());
        let const_cost = sfmt(const_cost);
        let var_cost = sfmt(var_cost);
        let var_all = sfmt(var_all);
        let all = sfmt(all);
        info!(
            "DataGroupPeriodic:\
            \n\tconst data size:\t{const_cost}\
            \n\tvariable data (per):\t{var_cost}\
            \n\tvariable size is:\t{var_set}\
            \n\tvarabile data size:\t{var_all}\
            \n\toverall data size:\t{all}"
        )
    }
}

impl Info for super::DataGroupSporadic {
    fn info() {
        let const_cost = 0;
        let var_cost = 8;
        let var_set = tuning::DATA_GROUP_SPORADIC_SIZE;
        let var_all = var_cost * var_set;
        let all = var_all + const_cost;
        info!(
            "DataGroupSporadic:\
            \n\tconst data size:\t{}\
            \n\tvariable data (per):\t{}\
            \n\tvariable size is:\t{var_set}\
            \n\tvariable data size:\t{}\
            \n\toverall data size:\t{}",
            sfmt(const_cost),
            sfmt(var_cost),
            sfmt(var_all),
            sfmt(all)
        )
    }
}

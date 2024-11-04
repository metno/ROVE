//! Generated proto bindings, plus handwritten trait impls for proto types

use crate::{data_switch, harness};

tonic::include_proto!("rove");

impl TryFrom<olympian::Flag> for Flag {
    type Error = String;

    fn try_from(item: olympian::Flag) -> Result<Self, Self::Error> {
        match item {
            olympian::Flag::Pass => Ok(Self::Pass),
            olympian::Flag::Fail => Ok(Self::Fail),
            olympian::Flag::Warn => Ok(Self::Warn),
            olympian::Flag::Inconclusive => Ok(Self::Inconclusive),
            olympian::Flag::Invalid => Ok(Self::Invalid),
            olympian::Flag::DataMissing => Ok(Self::DataMissing),
            olympian::Flag::Isolated => Ok(Self::Isolated),
            _ => Err(format!("{:?}", item)),
        }
    }
}

impl TryFrom<data_switch::Timeseries<olympian::Flag>> for FlagSeries {
    type Error = String;

    fn try_from(value: data_switch::Timeseries<olympian::Flag>) -> Result<Self, Self::Error> {
        let flags = value
            .values
            .into_iter()
            .map(|flag| {
                let flag: Flag = flag
                    .try_into()
                    .map_err(|e| format!("unrecognized flag: {:?}", e))?;
                Ok(flag.into())
            })
            .collect::<Result<Vec<i32>, String>>()?;
        Ok(Self {
            id: value.tag,
            flags,
        })
    }
}

impl TryFrom<harness::CheckResult> for CheckResult {
    type Error = String;

    fn try_from(value: harness::CheckResult) -> Result<Self, Self::Error> {
        Ok(Self {
            check: value.check,
            flag_series: value
                .results
                .into_iter()
                .map(|ts| ts.try_into())
                .collect::<Result<Vec<FlagSeries>, String>>()?,
        })
    }
}

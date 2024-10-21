use hidefix::prelude::*;

pub fn modelfetch(f: &str, vars: Vec<String>) -> anyhow::Result<()> {
    println!("Indexing file: {f}..");
    let i = Index::index(f)?;

    for var in vars {
        let mut r = i.reader(&var).unwrap();

        println!("Reading values from {var}..");
        let values = r.values::<f32, _>(..)?;

        println!("Number of values: {}", values.len());
        println!("First value: {}", values.first().unwrap());
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use geo::Point;
    use proj::{Proj, Transform};

    #[test]
    fn test_modelfetch() {
        let mut blindern = (18700, Point::new(10.720000, 59.942300));

        let wgs84 = "+proj=longlat +datum=WGS84";
        let lcc = "+proj=lcc +lat_0=63 +lon_0=15 +lat_1=63 +lat_2=63 +no_defs +R=6.371e+06";
        let wgs84_to_lcc = Proj::new_known_crs(&wgs84, &lcc, None).unwrap();

        blindern.1.transform(&wgs84_to_lcc).unwrap();

        println!("post-transformation point: {:?}", blindern.1);

        assert!(modelfetch("/lustre/storeB/project/metkl/klinogrid/range_check_thresholds/met_analysis_ltc_1_0km_nordic_tx1hour_mon_198101-202201.nc", vec!["tx1hour".to_string()]).is_ok());

        // TODO: get dimensions (corner coordinates, south-west corner, resolution (distance
        // steps)), take the difference from the reference corner to station location, and so get
        // the index, and extract the right data with it.
    }
}

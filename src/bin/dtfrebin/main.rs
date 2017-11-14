extern crate clap;
extern crate itertools;
extern crate byteorder;
extern crate dtf;

use clap::{Arg, App};
use std::collections::{HashMap, HashSet};
use std::cmp::{Ord, Ordering};
use itertools::Itertools;

///
/// converts orderbook updates to candles, and rebin 1 minute candles into 5min/ 12hour candles
/// 
fn main() {
        let matches = App::new("rebin")
                          .version("1.0.0")
                          .author("Ricky Han <tectonic@rickyhan.com>")
                          .about("rebin dtf files into candlesticks")
                          .arg(Arg::with_name("input")
                               .short("i")
                               .long("input")
                               .value_name("INPUT")
                               .help("file to read")
                               .required(true)
                               .takes_value(true))
                          .arg(Arg::with_name("aligned")
                               .short("a")
                               .long("aligned")
                               .help("Snap to grid"))
                          .arg(Arg::with_name("minutes")
                               .short("m")
                               .long("minutes")
                               .required(false)
                               .value_name("MINUTES")
                               .help("rebin into minutes. e.g. -m 60 # hour candles")
                               .takes_value(true))
                          .get_matches();

    let input = matches.value_of("input").unwrap();
    let aligned = matches.is_present("aligned");
    let minutes = matches.value_of("minutes").unwrap_or("1");

    let ups = dtf::decode(input);

    let candles = updates2candles(true, &ups);

    let rebinned = candles
                    .rebin(aligned, minutes.parse::<u16>().unwrap())
                    .unwrap()
                    .to_csv();

    println!("{}", rebinned);
}


fn updates2candles(fix_missing: bool, ups: &[dtf::Update]) -> Candles {
    let mut last_ts = 0;        // store the last timestep to test continuity
    let mut last_close = 0.;    // 

    let mut candles : HashMap<u32, Candle> = HashMap::new();

    for trade in ups.iter() {
        if !trade.is_trade { continue; }
        // floor(ts)
        let ts = (dtf::fill_digits(trade.ts) / 1000 / 60 * 60) as u32; 
        
        if fix_missing && (ts != last_ts + 60) && (last_ts != 0) && (last_ts != ts) {
            //insert continuation candle(s)
            let mut cur = last_ts;
            while cur < ts {
                candles.insert(cur, Candle {
                    time: cur,
                    volume: 0.,
                    high: last_close,
                    low: last_close,
                    open: last_close,
                    close: last_close
                });
                cur += 60;
            }
        }

        let new_candle = if candles.contains_key(&ts) {
            let c = candles.get(&ts).unwrap();

            Candle {
                time: ts,
                volume: c.volume + trade.size,
                high: if trade.price > c.high { trade.price } else { c.high },
                low: if trade.price < c.low  { trade.price } else { c.low },
                close: trade.price,
                open: c.open
            }

        } else {
            Candle {
                time: ts,
                volume: trade.size,
                high: trade.price,
                low: trade.price,
                close: trade.price,
                open: trade.price
            }
        };

        candles.insert(ts, new_candle);
        last_ts = ts;
        last_close = trade.price;
    }

    let mut vec_candle = candles
                        .drain()                    // random order, hence sort below
                        .map(|(_k,v)| v)
                        .collect::<Vec<Candle>>();

    vec_candle.sort();                              // XXX: IMPORTANT! DO NOT DELETE.

    return Candles::new(
        vec_candle,
        1
    );
}

#[derive(Clone, Debug, PartialEq)]
struct Candles {
    v: Vec<Candle>,
    scale: u16
}

impl Candles {
    fn to_csv(self) -> String {
        let csvs : Vec<String> = self.v.into_iter()
                .map(|candle| candle.to_csv())
                .collect();

        csvs.join("\n")
    }

    /// find missing epochs
    fn missing_epochs(&self) -> Vec<u32> {

        let mut set = HashSet::<u32>::new();
        let mut missing = Vec::<u32>::new();
        
        for candle in self.v.iter() {
            set.insert(candle.time);
        }

        let min_epoch = self.v.first().unwrap().time;
        let max_epoch = self.v.last().unwrap().time;

        let mut it = min_epoch;
        while it < max_epoch {
            if !set.contains(&it) {
                missing.push(it);
            }
            it += (self.scale as u32) * 60;
        }

        missing
    }

    /// return ranges of missing epochs
    fn missing_ranges(&self) -> Vec<(u32, u32)> {
        ranges(&self.missing_epochs())
    }

    /// insert continuation candles and fix missing
    fn insert_continuation_candles(&mut self) {
        let mut candles = Vec::new();
        let (mut last_ts, last_close) = {
            let first = self.v.first().unwrap();
            let last_ts = first.time;
            let last_close = first.close;
            (last_ts, last_close)
        };

        for candle in self.v.iter() {
            let ts = candle.time;
            if (ts != last_ts + 60) && (last_ts != 0) && (last_ts != ts) {
                //insert continuation candle(s)
                let mut cur = last_ts;
                while cur < ts {
                    candles.push(Candle {
                        time: cur,
                        volume: 0.,
                        high: last_close,
                        low: last_close,
                        open: last_close,
                        close: last_close
                    });
                    cur += 60;
                }
            }
            last_ts = ts;
        }

        self.v = candles;

    }


    /// create new Candles object
    fn new(v: Vec<Candle>, scale: u16) -> Candles {
        let ret = Candles {
            v,
            scale
        };

        // assert!(self._test_epochs_must_be_sequential());
        ret
    }

    /// epochs must be exactly incrementing by n * 60
    fn _test_epochs_must_be_sequential(&self) -> bool {
        // all([a[0] + i * 60 * minutes == x for i, x in enumerate(a)])
        let mut i : u32 = 0;
        let first = self.v.get(0).unwrap().time;
        for row in &self.v {
            if first + i * 60 * (self.scale as u32) != row.time {
                return false;
            }
            i += 1;
        }
        true
    }


    /// rebin 1 minute candles to x-minute candles
    fn rebin(self, align: bool, new_scale : u16) -> Option<Candles> {
        if new_scale < self.scale { return None }
        else if new_scale == self.scale { return Some(self) }

        let mut res = Vec::new();

        let mut startacc = 0;
        let mut openacc = 0.;
        let mut highacc = 0.;
        let mut lowacc = 0.;
        let mut volumeacc = 0.; 

        let mut aligned = false;
        let mut i = 0;

        for row in self.v.iter() {
            // align with minute mark ("snap" to grid)
            //
            //
            // --|------|------|------|-->
            //   |
            //   ^ discard up to this point         
            //
            //
            if align && !aligned {
                let snap_point = (row.time / (self.scale as u32 * 60)) * (self.scale as u32 * 60);
                if row.time == snap_point {
                    aligned = true;
                    i = 0;
                } else {
                    continue;
                }
            }
            // new candle, reset using first candle
            if i % new_scale as usize == 0 {
                startacc = row.time;
                openacc = row.open;
                highacc = row.high;
                lowacc = row.low;
                volumeacc = row.volume;
                i += 1;
                continue;
            }

            // accumulate new high, low and volume
            highacc =  if row.high > highacc {row.high} else {highacc};
            lowacc = if row.low < lowacc {row.low} else {lowacc};
            volumeacc += row.volume;

            // if it's the last minute, insert
            if (i % (new_scale as usize)) == ((new_scale as usize) - 1 ){
                let candle = Candle {
                    time: startacc,
                    open: openacc,
                    high: highacc,
                    low: lowacc,
                    close: row.close,
                    volume: volumeacc
                };
                
                res.push(candle);
            }
            i += 1;
        }

        assert_eq!(res.len(), self.v.len() / (new_scale as usize));
        assert!(self._test_epochs_must_be_sequential());

        Some(Candles {
            v: res,
            scale: new_scale
        })
    }
}

#[derive(PartialOrd, PartialEq, Clone, Debug)]
struct Candle {
    time:   u32,
    open:   f32,
    high:   f32,
    low:    f32,
    close:  f32,
    volume: f32,
}

impl Ord for Candle {
    fn cmp(&self, other : &Candle) -> Ordering {
        self.time.cmp(&other.time)
    }
}

impl Eq for Candle {}

impl Candle {
    fn to_csv(&self) -> String{
        format!("{},{},{},{},{},{}",
                self.time, self.open, self.high, self.low, self.close, self.volume)
    }
}

/// Check a list of sequence
/// Returns maximum continuous sequence
/// [1,2,3,5,6,7] -> [(1,3), (5,7)]
/// :param lst: list of epochs
/// :return: list of tuples of shape (start, end)
fn ranges(lst: &Vec<u32>) -> Vec<(u32, u32)>{
    let mut pos = Vec::new();

    for (i, j) in lst.iter().enumerate() {
        pos.push(j/ 60 - i as u32);
    }

    let mut ret = Vec::new();
    let mut t = 0;
    for (_i, els) in &pos.into_iter().group_by(|e| *e) {
        let l = els.count();
        let el = lst.get(t).unwrap();
        t += l;
        ret.push((el.clone(), el + 60 * (l-1) as u32));
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candle_to_csv() {
        let inp = Candle {
            time: 0,
            open: 0.,
            close: 0.,
            high: 0.,
            low: 0.,
            volume: 0.,
        };
        let target = "0,0,0,0,0,0";
        assert_eq!(inp.to_csv(), target);
    }

    #[test]
    fn test_candle_ord() {
        let c1 = Candle {
            time: 0,
            open: 10.,
            close: 10.,
            high: 10.,
            low: 10.,
            volume: 10.,
        };
        let c2 = Candle {
            time: 10,
            open: 0.,
            close: 0.,
            high: 0.,
            low: 0.,
            volume: 0.,
        };

        assert!(c1 < c2);
    }

    #[test]
    fn test_candle_snap_to_grid() {
        let mut v = Vec::new();
        for i in 30..121 {
            v.push(Candle {
                time: 60 * i,
                open: 0.,
                close: 1.,
                high: 2.,
                low: 0.,
                volume: 1.
            });
        }

        let candles = Candles::new(v, 1);

        assert_eq!(Candles {
            v: vec![Candle {
                time: 1800,
                open: 0.,
                high: 2.,
                low: 0.,
                close: 1.,
                volume: 60.
            }],
            scale: 60
        }, candles.rebin(true, 60).unwrap());
    }

    #[test]
    fn test_create_new_candles() {
        assert_eq!(Candles::new(vec![], 1), Candles {v: vec![], scale:1});
    }

    #[test]
    fn test_fix_missing_candles() {
        let mut v = Vec::new();
        for i in 30..121 {
            if i >= 50 && i <= 60 {
                continue;
            }

            v.push(Candle {
                time: 60 * i,
                open: 0.,
                close: 1.,
                high: 2.,
                low: 0.,
                volume: 1.
            });
        }
        let mut candles = Candles::new(v, 1);

        assert_eq!(vec![3000, 3060, 3120, 3180, 3240, 3300, 3360, 3420, 3480, 3540, 3600], candles.missing_epochs());
        assert_eq!(vec![(3000, 3600)], candles.missing_ranges());
        candles.insert_continuation_candles();
        assert_eq!(Vec::<u32>::new(), candles.missing_epochs());
        assert_eq!(Vec::<(u32,u32)>::new(), candles.missing_ranges());
    }


    #[test]
    fn test_ranges() {
        let v : Vec<u32> = vec![60,120,180,600,660,720];
        let result = ranges(&v);
        let shouldbe : Vec<(u32,u32)> = vec![(60,180), (600, 720)];
        assert_eq!(shouldbe, result);

        let v : Vec<u32> = vec![0,60,120,180,240,600,660,720];
        let result = ranges(&v);
        let shouldbe : Vec<(u32,u32)> = vec![(0,240), (600, 720)];
        assert_eq!(shouldbe, result);
    }

    #[test]
    fn test_must_be_sequential() {
        let mut candles : Vec<Candle> = Vec::new();
        for i in 1..10 {
            let j = i * 60;
            candles.push(Candle {
                time: j,
                open: 0.,
                close: 0.,
                high: 0.,
                low: 0.,
                volume: 0.
            });
        }

        let c = Candles { v: candles.clone(), scale: 1};
        assert!(c._test_epochs_must_be_sequential());

        candles.push(Candle {
            time: 10000,
            open: 0.,
            close: 0.,
            high: 0.,
            low: 0.,
            volume: 0.
        });
        let g = Candles { v: candles, scale: 1};
        assert!(!g._test_epochs_must_be_sequential());
    }

    #[test]
    fn test_rebin() {
        let mut candles : Vec<Candle> = Vec::new();
        let to_scale :usize= 5;
        let upto :usize = 5;
        for i in 1..(upto+1) {
            candles.push(Candle {
                time: i as u32 * 60,
                open: 0.,
                close: 0.,
                high: 0.,
                low: 0.,
                volume: 0.
            });
        }

        let c = Candles { v: candles.clone(), scale: 1};
        println!("{:?}", c);
        let rebinned = c.rebin(false, to_scale as u16).unwrap();
        println!("{:?}", rebinned);
        assert_eq!(rebinned.scale, to_scale as u16);
        assert_eq!(rebinned.v.len(), upto / to_scale);
    }

    #[test]
    fn should_have_right_attr() {
        let mut candles : Vec<Candle> = Vec::new();
        let to_scale :usize= 5;
        let upto :usize = 5;
        for i in 1..(upto+1) {
            candles.push(Candle {
                time: i as u32 * 60,
                open: 100.*i as f32,
                close: 100.*i as f32,
                high: i as f32,
                low: i as f32,
                volume: i as f32
            });
        }

        let c = Candles { v: candles.clone(), scale: 1};
        println!("{:?}", c);
        let rebinned = c.rebin(false, to_scale as u16).unwrap();
        println!("{:?}", rebinned);
        assert_eq!(rebinned.scale, to_scale as u16);
        assert_eq!(rebinned.v.len(), upto / to_scale);


        let mut i = 1;
        for bin in rebinned.v.iter() {
            println!("{:?}", bin);
            assert_eq!(bin.high, (i * to_scale) as f32);
            assert_eq!(bin.open, (100 * (i-1) * to_scale + 100) as f32);
            assert_eq!(bin.close, 100. * (i * to_scale) as f32);
            assert_eq!(bin.volume, (1+(i-1)*to_scale..(i*to_scale + 1)).fold(0, |a,b| a+b) as f32);
            i += 1;
        }
    }

}
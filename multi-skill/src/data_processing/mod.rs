mod cf_api;
mod ctf_api;
mod dataset;

pub use cf_api::fetch_cf_contest_list;
pub use dataset::{get_dataset_from_disk, CachedDataset, ClosureDataset, Dataset, Wrap};
use rand::seq::SliceRandom;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;

fn one() -> f64 {
    1.0
}

fn is_one(&weight: &f64) -> bool {
    weight == one()
}

/// Represents the outcome of a contest.
#[derive(Serialize, Deserialize, Clone)]
pub struct Contest {
    /// A human-readable title for the contest.
    pub name: String,
    /// The source URL, if any.
    pub url: Option<String>,
    /// The relative weight of a contest, default is 1.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub weight: f64,
    /// The number of seconds from the Unix Epoch to the end of the contest.
    pub time_seconds: u64,
    /// The list of standings, containing a name and the enclosing range of ties.
    pub standings: Vec<(String, usize, usize)>,
}

impl Contest {
    /// Create a contest with empty standings, useful for testing.
    pub fn new(index: usize) -> Self {
        Self {
            name: format!("Round #{}", index),
            url: None,
            weight: 1.,
            time_seconds: index as u64 * 86_400,
            standings: vec![],
        }
    }

    /// Returns the contestant's position, if they participated.
    pub fn find_contestant(&mut self, handle: &str) -> Option<usize> {
        self.standings.iter().position(|x| x.0 == handle)
    }

    /// Remove a contestant with the given handle, and return it if it exists.
    pub fn remove_contestant(&mut self, handle: &str) -> Option<(String, usize, usize)> {
        let pos = self.find_contestant(handle)?;
        let contestant = self.standings.remove(pos);
        for (_, lo, hi) in self.standings.iter_mut() {
            if *hi >= pos {
                *hi -= 1;
                if *lo > pos {
                    *lo -= 1;
                }
            }
        }
        Some(contestant)
    }

    /// Assuming `self.standings` is a subset of a valid standings list,
    /// corrects the `lo` and `hi` values to make the new list valid
    fn fix_lo_hi(&mut self) {
        self.standings.sort_unstable_by_key(|(_, lo, _)| *lo);
        let len = self.standings.len();
        let mut lo = 0;
        while lo < len {
            let mut hi = lo;
            while hi + 1 < len && self.standings[lo].1 == self.standings[hi + 1].1 {
                hi += 1;
            }
            for (_, st_lo, st_hi) in &mut self.standings[lo..=hi] {
                *st_lo = lo;
                *st_hi = hi;
            }
            lo = hi + 1;
        }
    }

    fn clone_with_standings(&self, standings: Vec<(String, usize, usize)>) -> Self {
        let mut contest = Self {
            name: self.name.clone(),
            url: self.url.clone(),
            weight: self.weight,
            time_seconds: self.time_seconds,
            standings,
        };
        contest.fix_lo_hi();
        contest
    }

    /// Split into random disjoint contests, each with at most n participants
    pub fn random_split<R: ?Sized + rand::Rng>(
        mut self,
        n: usize,
        rng: &mut R,
    ) -> impl Iterator<Item = Contest> {
        self.standings.shuffle(rng);
        let split_standings: Vec<_> = self.standings.chunks(n).map(<[_]>::to_vec).collect();
        split_standings
            .into_iter()
            .map(move |chunk| self.clone_with_standings(chunk))
    }

    /// Add a contestant with the given handle in last place.
    pub fn push_contestant(&mut self, handle: impl Into<String>) {
        let place = self.standings.len();
        self.standings.push((handle.into(), place, place));
    }
}

/// Compressed summary of a contest
#[derive(Serialize, Deserialize)]
pub struct ContestSummary {
    pub name: String,
    pub url: Option<String>,
    pub weight: f64,
    pub time_seconds: u64,
    pub num_contestants: usize,
}

impl ContestSummary {
    /// Returns a summary of the given contest, stripped of detailed standings
    pub fn new(contest: &Contest) -> Self {
        Self {
            name: contest.name.clone(),
            url: contest.url.clone(),
            weight: contest.weight,
            time_seconds: contest.time_seconds,
            num_contestants: contest.standings.len(),
        }
    }
}

pub fn write_to_json<T: Serialize + ?Sized>(
    value: &T,
    path: impl AsRef<Path>,
) -> Result<(), String> {
    let cached_json = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    std::fs::write(path.as_ref(), cached_json).map_err(|e| e.to_string())
}

fn write_to_csv<T: Serialize>(values: &[T], path: impl AsRef<Path>) -> Result<(), String> {
    let file = std::fs::File::create(path.as_ref()).map_err(|e| e.to_string())?;
    let mut writer = csv::Writer::from_writer(file);
    values
        .iter()
        .try_for_each(|val| writer.serialize(val))
        .map_err(|e| e.to_string())
}

pub fn write_slice_to_file_fallible<T: Serialize>(
    values: &[T],
    path: impl AsRef<Path>,
) -> Result<(), String> {
    let path = path.as_ref();
    match path.extension().and_then(|s| s.to_str()) {
        Some("json") => write_to_json(values, path),
        Some("csv") => write_to_csv(values, path),
        _ => Err("Invalid or missing filename extension".into()),
    }
}

pub fn write_slice_to_file<T: Serialize>(values: &[T], path: impl AsRef<Path>) {
    let path = path.as_ref();
    let write_res = write_slice_to_file_fallible(values, path);
    match write_res {
        Ok(()) => tracing::info!("Successfully wrote to {:?}", path),
        Err(msg) => tracing::error!("WARNING: failed write to {:?} because {}", path, msg),
    };
}

/// Helper function to get contest results from the Codeforces API.
pub fn get_dataset_from_codeforces_api(
    contest_id_file: impl AsRef<std::path::Path>,
) -> Wrap<impl Dataset<Item = Contest>> {
    let client = Client::new();
    let contests_json =
        std::fs::read_to_string(contest_id_file).expect("Failed to read contest IDs from file");
    let contest_ids: Vec<usize> = serde_json::from_str(&contests_json)
        .expect("Failed to parse JSON contest IDs as a Vec<usize>");

    Wrap::from_closure(contest_ids.len(), move |i| {
        cf_api::fetch_cf_contest(&client, contest_ids[i]).expect("Failed to fetch contest")
    })
}

/// Helper function to get contest results from the CTFtime API.
pub fn get_dataset_from_ctftime_api() -> Wrap<impl Dataset<Item = Contest>> {
    let contests = ctf_api::fetch_ctf_history();

    Wrap::from_closure(contests.len(), move |i| contests[i].clone())
}

pub type BoxedDataset<T> = Box<dyn Dataset<Item = T> + Send + Sync>;
pub type ContestDataset = Wrap<BoxedDataset<Contest>>;

/// Helper function to get any named dataset.
// TODO: actually throw errors when the directory is not found.
pub fn get_dataset_by_name(dataset_name: &str) -> Result<ContestDataset, String> {
    const CF_IDS: &str = "../data/codeforces/contest_ids.json";

    let dataset_dir = format!("../cache/{}", dataset_name);
    let dataset = if dataset_name == "codeforces" {
        // Rate-limit API calls so we don't burden Codeforces
        get_dataset_from_codeforces_api(CF_IDS)
            .rate_limit(std::time::Duration::from_millis(500))
            .cached(dataset_dir)
            .boxed()
    //} else if dataset_name == "ctf" {
    //    get_dataset_from_ctftime_api().cached(dataset_dir).boxed()
    } else {
        get_dataset_from_disk(dataset_dir).boxed()
    };
    Ok(dataset)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_codeforces_data() {
        let dataset = get_dataset_by_name("codeforces").unwrap();
        let first_contest = dataset.get(0);
        let first_winner = &first_contest.standings[0];

        assert_eq!(first_contest.weight, 1.);
        assert_eq!(first_contest.standings.len(), 66);
        assert_eq!(first_winner.0, "vepifanov");
        assert_eq!(first_winner.1, 0);
        assert_eq!(first_winner.2, 0);
    }
}

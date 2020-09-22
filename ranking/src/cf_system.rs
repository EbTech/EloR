use super::contest_config::Contest;
use super::compute_ratings::{RatingSystem, Rating, Player, robust_average};
use rayon::prelude::*;
use std::cell::{RefCell, RefMut};
use std::cmp::max;
use std::collections::{HashMap, VecDeque};

/// Codeforces system details: https://codeforces.com/blog/entry/20762
pub struct CodeforcesSystem {
    sig_perf: f64,
    weight: f64,
}

impl Default for CodeforcesSystem {
    fn default() -> Self {
        Self {
            sig_perf: 800. / std::f64::consts::LN_10,
            weight: 1.,
        }
    }
}

impl CodeforcesSystem {
    // ratings is a list of the participants, ordered from first to last place
    // returns: performance of the player in ratings[id] who tied against ratings[lo..hi]
    fn compute_performance(
        better: &[Rating],
        worse: &[Rating],
        all: &[Rating],
        my_rating: Rating,
    ) -> f64 {
        // The conversion is 2*rank - 1/my_sig = 2*pos_offset + tied_offset = pos - neg + all
        let pos_offset: f64 = better.iter().map(|rating| rating.sig.recip()).sum();
        let neg_offset: f64 = worse.iter().map(|rating| rating.sig.recip()).sum();
        let all_offset: f64 = all.iter().map(|rating| rating.sig.recip()).sum();

        let ac_rank = 0.5 * (pos_offset - neg_offset + all_offset + my_rating.sig.recip());
        let ex_rank = 0.5
            * (my_rating.sig.recip()
                + all
                    .iter()
                    .map(|rating| {
                        (1. + ((rating.mu - my_rating.mu) / rating.sig).tanh()) / rating.sig
                    })
                    .sum::<f64>());

        let geo_rank = (ac_rank * ex_rank).sqrt();
        let geo_offset = 2. * geo_rank - my_rating.sig.recip() - all_offset;
        let geo_rating = robust_average(all.iter().cloned(), geo_offset, 0.);
        geo_rating
    }
}

impl RatingSystem for CodeforcesSystem {
    fn round_update(&self, standings: Vec<(&mut Player, usize, usize)>) {
        let all_ratings: Vec<Rating> = standings
            .par_iter()
            .map(|(player, _, _)| Rating {
                mu: player.approx_posterior.mu,
                sig: self.sig_perf,
            })
            .collect();

        standings
            .into_par_iter()
            .zip(all_ratings.par_iter())
            .for_each(|((player, lo, hi), &my_rating)| {
                let geo_perf = Self::compute_performance(
                    &all_ratings[..lo],
                    &all_ratings[hi + 1..],
                    &all_ratings,
                    my_rating,
                );
                let player_mu = &mut player.approx_posterior.mu;
                *player_mu = (*player_mu + self.weight * geo_perf) / (1. + self.weight);
            });
    }
}
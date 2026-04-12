use nih_plug::prelude::*;
use rand::Rng;

use crate::params::SlammerParams;

pub const LOCK_SUB: u8 = 1 << 0;
pub const LOCK_MID: u8 = 1 << 1;
pub const LOCK_TOP: u8 = 1 << 2;
pub const LOCK_SAT: u8 = 1 << 3;
pub const LOCK_EQ: u8 = 1 << 4;
pub const LOCK_COMP: u8 = 1 << 5;

pub fn randomize(setter: &ParamSetter, params: &SlammerParams, locked: u8) {
    let mut rng = rand::thread_rng();

    macro_rules! rand_float {
        ($param:expr) => {
            setter.begin_set_parameter(&$param);
            setter.set_parameter_normalized(&$param, rng.gen::<f32>());
            setter.end_set_parameter(&$param);
        };
    }

    macro_rules! rand_float_biased {
        ($param:expr) => {
            setter.begin_set_parameter(&$param);
            setter.set_parameter_normalized(&$param, rng.gen::<f32>().powf(0.7));
            setter.end_set_parameter(&$param);
        };
    }

    macro_rules! rand_bool {
        ($param:expr, $chance:expr) => {
            setter.begin_set_parameter(&$param);
            setter.set_parameter(&$param, rng.gen::<f32>() < $chance);
            setter.end_set_parameter(&$param);
        };
    }

    // Global envelope params — not specific to any one layer, always randomize.
    rand_float!(params.decay_ms);
    rand_float!(params.drift_amount);

    if locked & LOCK_SUB == 0 {
        rand_float_biased!(params.sub_gain);
        rand_float!(params.sub_fstart);
        rand_float!(params.sub_fend);
        rand_float!(params.sub_sweep_ms);
        rand_float!(params.sub_sweep_curve);
    }

    if locked & LOCK_MID == 0 {
        rand_float_biased!(params.mid_gain);
        rand_float!(params.mid_fstart);
        rand_float!(params.mid_fend);
        rand_float!(params.mid_sweep_ms);
        rand_float!(params.mid_sweep_curve);
        rand_float!(params.mid_phase_offset);
        rand_float!(params.mid_decay_ms);
        rand_float_biased!(params.mid_tone_gain);
        rand_float_biased!(params.mid_noise_gain);
        rand_float!(params.mid_noise_color);
        rand_bool!(params.clap_on, 0.3);
        rand_float_biased!(params.clap_level);
        rand_float!(params.clap_freq);
        rand_float!(params.clap_tail_ms);
    }

    if locked & LOCK_TOP == 0 {
        rand_float_biased!(params.top_gain);
        rand_float!(params.top_decay_ms);
        rand_float!(params.top_freq);
        rand_float!(params.top_bw);
        rand_float!(params.top_metal);
    }

    if locked & LOCK_SAT == 0 {
        rand_float!(params.sat_mode);
        rand_float_biased!(params.sat_drive);
        rand_float_biased!(params.sat_mix);
    }

    if locked & LOCK_EQ == 0 {
        rand_float!(params.eq_tilt_db);
        rand_float!(params.eq_low_boost_db);
        rand_float!(params.eq_notch_freq);
        rand_float!(params.eq_notch_q);
        rand_float!(params.eq_notch_depth_db);
    }

    if locked & LOCK_COMP == 0 {
        rand_float!(params.comp_amount);
        rand_float!(params.comp_react);
        rand_float_biased!(params.comp_drive);
        rand_bool!(params.comp_limit_on, 0.3);
        rand_float!(params.comp_atk_ms);
        rand_float!(params.comp_rel_ms);
        rand_float!(params.comp_knee_db);
        rand_float!(params.dj_filter_pos);
        rand_float!(params.dj_filter_res);
        rand_bool!(params.dj_filter_pre, 0.3);
    }
}

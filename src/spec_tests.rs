//! Small executable contracts from the May 1994 CD-i specification:
//! IV.4.3 / Fig. IV.16-17 and IV.5.3. Synthetic inputs only.
//! Deliberately uses integral predictor outputs: fractional rounding behavior
//! and invalid coding parameters require separate hardware evidence.
use crate::audio::ChannelState;

#[test]
fn valid_adpcm_ranges_preserve_signed_gain_for_both_sample_widths() {
    // IV.5.3: Level A uses value * 2^(8-R), B/C value * 2^(12-R).
    for (bits, max_range) in [(8, 8), (4, 12)] {
        for range in 0..=max_range {
            for value in -(1i32 << (bits - 1))..(1 << (bits - 1)) {
                let normalized = (value * (1 << (16 - bits))) as i16;
                let expected = value * (1 << (16 - bits - range));
                let mut state = ChannelState::default();
                assert_eq!(
                    state.decode_xa_unit(range as u8, normalized) as i32,
                    expected,
                    "bits={bits} range={range} value={value}"
                );
            }
        }
    }
}

#[test]
fn four_predictors_match_documented_gains_and_sample_history() {
    // Exact integer basis vectors for K0/K1, scaled by a history sample of 256.
    for (filter, first, second) in [(0, 0, 0), (1, 240, 0), (2, 460, -208), (3, 392, -220)] {
        for (s0, s1, expected) in [
            (256, 0, first),
            (0, 256, second),
            (256, 256, first + second),
        ] {
            let mut state = ChannelState { s0, s1 };
            let out = state.decode_xa_unit(filter << 4, 0);
            assert_eq!(out, expected);
            assert_eq!((state.s0, state.s1), (expected, s0));
        }
    }
}

#[test]
fn decoder_saturates_both_signed_extremes_instead_of_wrapping() {
    for (sign, expected) in [(1, i16::MAX), (-1, i16::MIN)] {
        let mut state = ChannelState {
            s0: 30000 * sign,
            s1: -30000 * sign,
        };
        assert_eq!(state.decode_xa_unit(0x20, 0), expected);
    }
}

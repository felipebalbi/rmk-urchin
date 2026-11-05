# Approved status-screen pixels

These seven 68×160, one-bit fixtures were exported from the independently
implemented browser prototype after visual approval of layout A in dark polarity,
including the shorter advertising arcs and two 2×2 square marks.

Pixels are portrait row-major, packed MSB-first with no padding between rows.
Each file is 1,360 bytes. Set bits are light content on a dark background.
The examples use Base, profile 0, local battery 65%, and no USB power, except
the two explicit unknown-battery variants. They do not encode panel wire framing.

The tests compare the real Rust renderer against these reference pixels, then
separately check partial packets and the reconstructed panel contents.

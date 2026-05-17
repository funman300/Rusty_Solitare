# QS.png — Asset Content Bug

**Status:** Needs manual fix — replace `QS.png` with correct artwork.

**Symptom:** The Queen of Spades card renders with a diamond watermark baked
into the PNG artwork, while the top-left Android overlay correctly shows "Q♠".

**Diagnosis:**
- The code-side mapping (`card_face_asset_path(Rank::Queen, Suit::Spades)`)
  correctly returns `"cards/faces/classic/QS.png"` — confirmed by unit test.
- `QS.png` and `QD.png` have distinct MD5 hashes, so they are not the same
  file. The bug is in the pixel content of `QS.png` itself.

**Fix:** Replace `QS.png` with a correctly-drawn Queen of Spades image (spade
watermark, not diamond). The image should be 120×168 px to match every other
card face in this directory.

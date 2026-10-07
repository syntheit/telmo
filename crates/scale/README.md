# telmo-scale

Weigh small objects on a Force Touch trackpad. Rest a finger on the trackpad,
then set the object on it and keep touching. `space` zeroes, `u` switches
between grams and ounces. macOS only; on other systems it says so.

The idea and the calibration come from
[TrackWeight](https://github.com/KrishKrosh/TrackWeight) by Krish Shah (MIT),
which in turn uses [OpenMultitouchSupport](https://github.com/Kyome22/OpenMultitouchSupport)
by Takuto Nakamura (MIT). Like TrackWeight we read the `pressure` field of the
first touch from the private MultitouchSupport framework; that value is already
in grams, so no scaling is applied. The trackpad only reports pressure while it
senses a touch, hence the finger. Readings are approximate (a few grams of
drift, metal objects may count as a finger).

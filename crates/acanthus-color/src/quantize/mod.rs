pub use point_provider::PointProvider;
pub use point_provider_lab::PointProviderLab;
pub use quantizer::Quantizer;
pub use quantizer::QuantizerResult;
pub use quantizer_celebi::QuantizerCelebi;
pub use quantizer_map::QuantizerMap;
pub use quantizer_wsmeans::QuantizerWsmeans;
pub use quantizer_wu::QuantizerWu;

/// Colour distance and conversion for quantizers.
pub mod point_provider;
/// CIE Lab point provider.
pub mod point_provider_lab;
/// The quantizer trait and result type.
pub mod quantizer;
/// Wu quantization refined by weighted k-means.
pub mod quantizer_celebi;
/// Exact colour counting.
pub mod quantizer_map;
/// Weighted k-means refinement of cluster centres.
pub mod quantizer_wsmeans;
/// Wu box-splitting quantization.
pub mod quantizer_wu;

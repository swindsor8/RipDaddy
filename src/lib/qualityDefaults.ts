import type {
  AudioMode,
  DeliveryTarget,
  QualityProfile,
  ResolutionTier,
  SubtitleMode,
  VideoEncoder,
} from "./types";

export const RESOLUTION_TIERS: {
  id: ResolutionTier;
  shortLabel: string;
  dims: string;
}[] = [
  { id: "p480", shortLabel: "480p", dims: "720×480" },
  { id: "p720", shortLabel: "720p", dims: "1280×720" },
  { id: "p1080", shortLabel: "1080p", dims: "1920×1080" },
  { id: "uhd4k", shortLabel: "4K", dims: "3840×2160" },
];

export const VIDEO_ENCODERS: { id: VideoEncoder; label: string }[] = [
  { id: "x26510Bit", label: "x265 10-bit (HDR / Best Efficiency)" },
  { id: "x265", label: "x265 8-bit (HEVC)" },
  { id: "x264", label: "x264 (Max Compatibility)" },
  { id: "svtAv110Bit", label: "SVT-AV1 10-bit" },
  { id: "nvencH265", label: "NVIDIA NVENC H.265 (GPU)" },
  { id: "vaapiH265", label: "Linux VAAPI H.265 (GPU)" },
  { id: "qsvH265", label: "Intel QuickSync H.265 (GPU)" },
  { id: "vtH265", label: "Apple VideoToolbox H.265 (GPU)" },
];

export const ENCODER_PRESETS = [
  "ultrafast",
  "veryfast",
  "fast",
  "medium",
  "slow",
  "slower",
];

export const AUDIO_BITRATES = [128, 160, 192, 256, 320, 384];

export function qualityProfileKey(
  resolution: ResolutionTier,
  delivery: DeliveryTarget,
): string {
  return `${resolution}:${delivery}`;
}

export function shortEncoderLabel(encoder: VideoEncoder): string {
  switch (encoder) {
    case "x26510Bit":
      return "x265 10-bit";
    case "x265":
      return "x265";
    case "x264":
      return "x264";
    case "svtAv110Bit":
      return "AV1 10-bit";
    case "nvencH265":
      return "NVENC H.265";
    case "vaapiH265":
      return "VAAPI H.265";
    case "qsvH265":
      return "QSV H.265";
    case "vtH265":
      return "VT H.265";
  }
}

export function shortResolutionLabel(resolution: ResolutionTier): string {
  switch (resolution) {
    case "p480":
      return "480p";
    case "p720":
      return "720p";
    case "p1080":
      return "1080p";
    case "uhd4k":
      return "4K";
  }
}

export function shortAudioLabel(mode: AudioMode): string {
  switch (mode) {
    case "passthrough":
      return "Passthrough";
    case "aac51":
      return "5.1 AAC";
    case "aacStereo":
      return "Stereo AAC";
  }
}

export function rfQualityTierLabel(rf: number): string {
  if (rf <= 18.5) return "Archival / Transparent";
  if (rf <= 21.5) return "High Quality";
  if (rf <= 24.0) return "Balanced Streaming";
  return "Compact / Small File";
}

export function isValidEncoder(value: string): value is VideoEncoder {
  return VIDEO_ENCODERS.some((e) => e.id === value);
}

export function isValidAudioMode(value: string): value is AudioMode {
  return value === "passthrough" || value === "aac51" || value === "aacStereo";
}

export function isValidSubtitleMode(value: string): value is SubtitleMode {
  return value === "all" || value === "foreignScan" || value === "none";
}

/**
 * Fallback 8-slot Resolution × Delivery matrix mirroring `handbrake::recommended_quality_profiles`.
 */
export function defaultQualityProfileMap(): Record<string, QualityProfile> {
  const list: QualityProfile[] = [
    {
      id: "p480:local",
      label: "480p Local (DVD Archive)",
      resolution: "p480",
      delivery: "local",
      encoder: "x26510Bit",
      encoderPreset: "slow",
      rfQuality: 19,
      audioMode: "passthrough",
      audioBitrateKbps: 192,
      container: "mkv",
      decomb: true,
      webOptimize: false,
      subtitleMode: "all",
      estimatedBitrateLabel: "~1.2–2.2 Mbps",
      extraArgs: [],
    },
    {
      id: "p480:remote",
      label: "480p Remote (Low Bandwidth)",
      resolution: "p480",
      delivery: "remote",
      encoder: "x264",
      encoderPreset: "medium",
      rfQuality: 22,
      audioMode: "aacStereo",
      audioBitrateKbps: 128,
      container: "mp4",
      decomb: true,
      webOptimize: true,
      subtitleMode: "foreignScan",
      estimatedBitrateLabel: "~0.8–1.5 Mbps",
      extraArgs: [],
    },
    {
      id: "p720:local",
      label: "720p Local (HD Direct Play)",
      resolution: "p720",
      delivery: "local",
      encoder: "x26510Bit",
      encoderPreset: "slow",
      rfQuality: 19,
      audioMode: "passthrough",
      audioBitrateKbps: 256,
      container: "mkv",
      decomb: false,
      webOptimize: false,
      subtitleMode: "all",
      estimatedBitrateLabel: "~2.5–4.5 Mbps",
      extraArgs: [],
    },
    {
      id: "p720:remote",
      label: "720p Remote (Mobile / WAN)",
      resolution: "p720",
      delivery: "remote",
      encoder: "x264",
      encoderPreset: "medium",
      rfQuality: 23,
      audioMode: "aacStereo",
      audioBitrateKbps: 160,
      container: "mp4",
      decomb: false,
      webOptimize: true,
      subtitleMode: "foreignScan",
      estimatedBitrateLabel: "~1.5–2.8 Mbps",
      extraArgs: [],
    },
    {
      id: "p1080:local",
      label: "1080p Local (Blu-ray Quality)",
      resolution: "p1080",
      delivery: "local",
      encoder: "x26510Bit",
      encoderPreset: "slow",
      rfQuality: 18,
      audioMode: "passthrough",
      audioBitrateKbps: 320,
      container: "mkv",
      decomb: false,
      webOptimize: false,
      subtitleMode: "all",
      estimatedBitrateLabel: "~6–12 Mbps",
      extraArgs: [],
    },
    {
      id: "p1080:remote",
      label: "1080p Remote (Direct Stream)",
      resolution: "p1080",
      delivery: "remote",
      encoder: "x26510Bit",
      encoderPreset: "medium",
      rfQuality: 23,
      audioMode: "aac51",
      audioBitrateKbps: 256,
      container: "mp4",
      decomb: false,
      webOptimize: true,
      subtitleMode: "foreignScan",
      estimatedBitrateLabel: "~3–5 Mbps",
      extraArgs: [],
    },
    {
      id: "uhd4k:local",
      label: "4K Local (UHD 10-bit HDR)",
      resolution: "uhd4k",
      delivery: "local",
      encoder: "x26510Bit",
      encoderPreset: "slow",
      rfQuality: 18,
      audioMode: "passthrough",
      audioBitrateKbps: 384,
      container: "mkv",
      decomb: false,
      webOptimize: false,
      subtitleMode: "all",
      estimatedBitrateLabel: "~16–28 Mbps",
      extraArgs: [],
    },
    {
      id: "uhd4k:remote",
      label: "4K Remote (Efficient UHD)",
      resolution: "uhd4k",
      delivery: "remote",
      encoder: "x26510Bit",
      encoderPreset: "medium",
      rfQuality: 24,
      audioMode: "aac51",
      audioBitrateKbps: 320,
      container: "mp4",
      decomb: false,
      webOptimize: true,
      subtitleMode: "foreignScan",
      estimatedBitrateLabel: "~8–14 Mbps",
      extraArgs: [],
    },
  ];

  const map: Record<string, QualityProfile> = {};
  for (const item of list) {
    map[item.id] = item;
  }
  return map;
}

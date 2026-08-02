export const DEFAULT_WALLPAPER = "blue-folds";

export const WALLPAPER_URLS = {
  "liquid-only": null,
  "blue-folds": new URL(
    "./assets/canvas-wallpaper-blue-folds.webp",
    import.meta.url,
  ).href,
  "ember-dunes": new URL(
    "./assets/canvas-wallpaper-ember-dunes.webp",
    import.meta.url,
  ).href,
  "violet-tide": new URL(
    "./assets/canvas-wallpaper-violet-tide.webp",
    import.meta.url,
  ).href,
  "jade-ripples": new URL(
    "./assets/canvas-wallpaper-jade-ripples.webp",
    import.meta.url,
  ).href,
  "aurora-mist": new URL(
    "./assets/canvas-wallpaper-aurora-mist.webp",
    import.meta.url,
  ).href,
  "golden-strata": new URL(
    "./assets/canvas-wallpaper-golden-strata.webp",
    import.meta.url,
  ).href,
} as const;

export type WallpaperSlug = keyof typeof WALLPAPER_URLS;

export const WALLPAPERS: ReadonlyArray<{
  slug: WallpaperSlug;
  label: string;
}> = [
  { slug: "liquid-only", label: "Kun liquid glass" },
  { slug: "blue-folds", label: "Blå folder" },
  { slug: "ember-dunes", label: "Glødende klitter" },
  { slug: "violet-tide", label: "Violet tidevand" },
  { slug: "jade-ripples", label: "Jade-ringe" },
  { slug: "aurora-mist", label: "Aurora-dis" },
  { slug: "golden-strata", label: "Gyldne lag" },
];

export function isWallpaperSlug(
  slug: string | null | undefined,
): slug is WallpaperSlug {
  return slug != null && Object.hasOwn(WALLPAPER_URLS, slug);
}

export function resolveWallpaperUrl(slug?: string | null): string | null {
  return isWallpaperSlug(slug)
    ? WALLPAPER_URLS[slug]
    : WALLPAPER_URLS[DEFAULT_WALLPAPER];
}

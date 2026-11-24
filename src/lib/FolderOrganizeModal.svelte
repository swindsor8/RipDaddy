<script lang="ts">
  import type {
    EpisodePreviewSample,
    FolderContentKind,
    FolderOrganization,
    NamingConvention,
  } from "./types";

  interface Props {
    open: boolean;
    batchKind: FolderContentKind;
    showTitle: string;
    namingConvention: NamingConvention;
    outputExtension: string;
    outputDir: string;
    sourceFolderPath: string | null;
    selectedFileCount: number;
    detectedSeasons: number[];
    sampleItems: EpisodePreviewSample[];
    recommendedOrg: FolderOrganization;
    onBrowseOutputDir: () => void;
    onUseSourceFolderAsOutput: () => void;
    onConfirm: (org: FolderOrganization, saveAsDefault: boolean) => void;
    onCancel: () => void;
  }

  let {
    open,
    batchKind,
    showTitle,
    namingConvention,
    outputExtension,
    outputDir,
    sourceFolderPath,
    selectedFileCount,
    detectedSeasons,
    sampleItems,
    recommendedOrg,
    onBrowseOutputDir,
    onUseSourceFolderAsOutput,
    onConfirm,
    onCancel,
  }: Props = $props();

  let selectedOrg = $state<FolderOrganization>("showAndSeason");
  let saveAsDefault = $state(false);

  $effect(() => {
    if (open) {
      selectedOrg = recommendedOrg;
      saveAsDefault = false;
    }
  });

  function sanitizeName(raw: string): string {
    return (raw.trim() || "Untitled")
      .replace(/[<>:"/\\|?*]/g, " ")
      .replace(/\s+/g, " ")
      .replace(/\.+$/, "")
      .trim();
  }

  function formatSamplePath(
    sample: EpisodePreviewSample,
    org: FolderOrganization,
  ): string {
    const clean = sanitizeName(sample.title);
    const baseName = sample.year ? `${clean} (${sample.year})` : clean;
    const taggedBase =
      sample.providerTag && namingConvention !== "plain"
        ? `${baseName} ${sample.providerTag}`
        : baseName;

    if (sample.kind === "movie") {
      if (org === "flat") {
        return `${taggedBase}.${outputExtension}`;
      }
      return `${taggedBase}/${taggedBase}.${outputExtension}`;
    }

    const s = String(sample.season ?? 1).padStart(2, "0");
    const e = String(sample.episode ?? 1).padStart(2, "0");
    const epPart = sample.episodeTitle
      ? ` - ${sanitizeName(sample.episodeTitle)}`
      : "";
    const filename = `${baseName} - S${s}E${e}${epPart}.${outputExtension}`;
    switch (org) {
      case "showAndSeason":
        return `${taggedBase}/Season ${s}/${filename}`;
      case "seasonOnly":
        return `Season ${s}/${filename}`;
      case "flat":
        return filename;
    }
  }

  function folderBaseName(path: string): string {
    const trimmed = path.replace(/[\\/]+$/, "");
    const parts = trimmed.split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

  function handleWindowKeydown(event: KeyboardEvent): void {
    if (open && event.key === "Escape") {
      onCancel();
    }
  }
</script>

<svelte:window onkeydown={handleWindowKeydown} />

{#if open}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/25 p-4 backdrop-blur-[2px]"
  >
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Organize output folders"
      class="flex w-full max-w-[610px] flex-col overflow-hidden rounded-2xl border border-[#DAD3C7] bg-white shadow-[0_28px_70px_rgba(26,21,19,0.18)]"
    >
      <div
        class="h-[3px] w-full shrink-0 bg-gradient-to-r from-[#4CDE99] via-[#F8B142] to-[#FA637C]"
        aria-hidden="true"
      ></div>
      <!-- Header -->
      <header class="border-b border-[#E6E0D6] bg-[#FAF8F4] px-6 py-4">
        <div class="flex items-center justify-between gap-2">
          <h2 class="text-sm font-extrabold tracking-[-0.02em] text-[#1A1513]">
            {#if batchKind === "movies"}
              Organize movies into individual folders?
            {:else if batchKind === "mixed"}
              Organize Movies &amp; TV Season folders?
            {:else}
              Organize output into Season folders?
            {/if}
          </h2>

          {#if batchKind === "movies"}
            <span
              class="rounded-full border border-[#1FA868] bg-[#E6FBF1] px-2.5 py-0.5 font-mono text-[10px] font-bold text-[#0E4D31]"
            >
              {selectedFileCount} Movie{selectedFileCount === 1 ? "" : "s"}
            </span>
          {:else if batchKind === "mixed"}
            <span
              class="rounded-full border border-[#D68D1C] bg-[#FFF7E6] px-2.5 py-0.5 font-mono text-[10px] font-bold text-[#5E3906]"
            >
              Mixed Movies + TV
            </span>
          {:else if detectedSeasons.length > 1}
            <span
              class="rounded-full border border-[#1C8EC0] bg-[#E6F6FD] px-2.5 py-0.5 font-mono text-[10px] font-bold text-[#0E5678]"
            >
              {detectedSeasons.length} Seasons ({detectedSeasons
                .map((s) => `S${String(s).padStart(2, "0")}`)
                .join(", ")})
            </span>
          {:else if detectedSeasons.length === 1}
            <span
              class="rounded-full border border-[#E6E0D6] bg-white px-2 py-0.5 font-mono text-[10px] font-semibold text-[#61554F]"
            >
              Season {String(detectedSeasons[0]).padStart(2, "0")}
            </span>
          {/if}
        </div>
        <p class="mt-1 text-xs leading-relaxed text-[#61554F]">
          {#if batchKind === "movies"}
            Choose whether each encoded movie gets its own Plex/Jellyfin folder
            (<span class="font-mono text-[#1A1513]">Movie (Year)/Movie (Year).ext</span>)
            or writes flat into your output folder.
          {:else if batchKind === "mixed"}
            Both movies and TV episodes are queued. Choose whether RipDaddy should
            automatically create movie folders and <span class="font-mono text-[#1A1513]">Season XX</span>
            subfolders as it encodes, or write everything flat.
          {:else if detectedSeasons.length > 1}
            Multiple seasons of <span class="font-semibold text-[#1A1513]">{showTitle || "this show"}</span>
            are queued together. Choose whether RipDaddy should automatically
            create <span class="font-mono text-[#1A1513]">Season XX</span> folders
            as each episode encodes, or keep files flat.
          {:else}
            Choose how RipDaddy should place the {selectedFileCount} encoded
            episode{selectedFileCount === 1 ? "" : "s"} inside your output folder.
          {/if}
        </p>
      </header>

      <!-- Body -->
      <div class="space-y-3.5 p-6">
        <!-- Output Directory Picker Strip -->
        <div
          class="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-[#E6E0D6] bg-[#F7F4EE] px-3 py-2 text-[11px]"
        >
          <div class="min-w-0 flex-1">
            <span class="font-bold uppercase tracking-[0.06em] text-[#7D716A]">
              Output Folder:
            </span>
            <p class="truncate font-mono text-[#1A1513]" title={outputDir}>
              {outputDir}
            </p>
          </div>
          <div class="flex shrink-0 items-center gap-1.5">
            {#if sourceFolderPath && sourceFolderPath !== outputDir}
              <button
                type="button"
                onclick={() => {
                  onUseSourceFolderAsOutput();
                  selectedOrg = batchKind === "tvSeries" ? "seasonOnly" : "showAndSeason";
                }}
                title={`Output directly inside ${sourceFolderPath}`}
                class="rounded border border-[#9CD9F5] bg-[#E6F6FD] px-2 py-1 text-[10px] font-semibold text-[#0E5678] hover:bg-[#D1EFFB]"
              >
                Use "{folderBaseName(sourceFolderPath)}"
              </button>
            {/if}
            <button
              type="button"
              onclick={onBrowseOutputDir}
              class="rounded border border-[#DAD3C7] bg-white px-2 py-1 text-[10px] font-semibold text-[#1A1513] hover:bg-[#F2EDE5]"
            >
              Change...
            </button>
          </div>
        </div>

        <!-- Organization Cards with Live Path Previews -->
        <div class="space-y-2">
          <!-- Option 1: Full Organized Folders -->
          <button
            type="button"
            onclick={() => (selectedOrg = "showAndSeason")}
            class={`w-full rounded-xl border p-3.5 text-left transition ${
              selectedOrg === "showAndSeason"
                ? "border-[#11824F] bg-[#EEFCF5] ring-1 ring-[#11824F]"
                : "border-[#E6E0D6] bg-white hover:border-[#C4BCB0]"
            }`}
          >
            <div class="flex items-center justify-between gap-2">
              <div class="flex items-center gap-2">
                <span
                  class={`flex h-4 w-4 items-center justify-center rounded-full border ${
                    selectedOrg === "showAndSeason"
                      ? "border-[#11824F] bg-[#11824F]"
                      : "border-[#C4BCB0] bg-white"
                  }`}
                >
                  {#if selectedOrg === "showAndSeason"}
                    <span class="h-1.5 w-1.5 rounded-full bg-white"></span>
                  {/if}
                </span>
                <span class="text-xs font-bold text-[#1A1513]">
                  {#if batchKind === "movies"}
                    Create a folder for each Movie
                  {:else if batchKind === "mixed"}
                    Organize into Movie folders &amp; Show / Season folders
                  {:else}
                    Organize into Show &amp; Season folders
                  {/if}
                </span>
              </div>
              {#if recommendedOrg === "showAndSeason"}
                <span
                  class="rounded-full border border-[#1FA868] bg-[#E6FBF1] px-2 py-0.5 text-[9px] font-bold uppercase text-[#0E4D31]"
                >
                  Recommended
                </span>
              {/if}
            </div>
            <p class="mt-1 pl-6 text-[11px] text-[#70645D]">
              {#if batchKind === "movies"}
                Plex &amp; Jellyfin standard — each movie gets its own clean subfolder.
              {:else}
                Creates organized media folders and Season subfolders inside your output library.
              {/if}
            </p>
            <div
              class="mt-2 ml-6 space-y-0.5 rounded-md border border-[#E6E0D6] bg-white px-2.5 py-1.5 font-mono text-[10px] text-[#3F3530]"
            >
              {#each sampleItems as sample, idx (`full-${sample.kind}-${sample.title}-${sample.season ?? 0}-${sample.episode ?? idx}`)}
                <p class="truncate">{formatSamplePath(sample, "showAndSeason")}</p>
              {/each}
            </div>
          </button>

          <!-- Option 2: Season Folders Only (TV / Mixed only) -->
          {#if batchKind !== "movies"}
            <button
              type="button"
              onclick={() => (selectedOrg = "seasonOnly")}
              class={`w-full rounded-xl border p-3.5 text-left transition ${
                selectedOrg === "seasonOnly"
                  ? "border-[#11824F] bg-[#EEFCF5] ring-1 ring-[#11824F]"
                  : "border-[#E6E0D6] bg-white hover:border-[#C4BCB0]"
              }`}
            >
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-2">
                  <span
                    class={`flex h-4 w-4 items-center justify-center rounded-full border ${
                      selectedOrg === "seasonOnly"
                        ? "border-[#11824F] bg-[#11824F]"
                        : "border-[#C4BCB0] bg-white"
                    }`}
                  >
                    {#if selectedOrg === "seasonOnly"}
                      <span class="h-1.5 w-1.5 rounded-full bg-white"></span>
                    {/if}
                  </span>
                  <span class="text-xs font-bold text-[#1A1513]">
                    Create Season folders directly inside Output folder
                  </span>
                </div>
                {#if recommendedOrg === "seasonOnly"}
                  <span
                    class="rounded-full border border-[#1FA868] bg-[#E6FBF1] px-2 py-0.5 text-[9px] font-bold uppercase text-[#0E4D31]"
                  >
                    Recommended
                  </span>
                {/if}
              </div>
              <p class="mt-1 pl-6 text-[11px] text-[#70645D]">
                Ideal when your output folder is already named "{showTitle || "the show"}" — avoids nesting the show name twice.
              </p>
              <div
                class="mt-2 ml-6 space-y-0.5 rounded-md border border-[#E6E0D6] bg-white px-2.5 py-1.5 font-mono text-[10px] text-[#3F3530]"
              >
                {#each sampleItems as sample, idx (`season-${sample.kind}-${sample.title}-${sample.season ?? 0}-${sample.episode ?? idx}`)}
                  <p class="truncate">{formatSamplePath(sample, "seasonOnly")}</p>
                {/each}
              </div>
            </button>
          {/if}

          <!-- Option 3: Keep Flat -->
          <button
            type="button"
            onclick={() => (selectedOrg = "flat")}
            class={`w-full rounded-xl border p-3.5 text-left transition ${
              selectedOrg === "flat"
                ? "border-[#11824F] bg-[#EEFCF5] ring-1 ring-[#11824F]"
                : "border-[#E6E0D6] bg-white hover:border-[#C4BCB0]"
            }`}
          >
            <div class="flex items-center justify-between gap-2">
              <div class="flex items-center gap-2">
                <span
                  class={`flex h-4 w-4 items-center justify-center rounded-full border ${
                    selectedOrg === "flat"
                      ? "border-[#11824F] bg-[#11824F]"
                      : "border-[#C4BCB0] bg-white"
                  }`}
                >
                  {#if selectedOrg === "flat"}
                    <span class="h-1.5 w-1.5 rounded-full bg-white"></span>
                  {/if}
                </span>
                <span class="text-xs font-bold text-[#1A1513]">
                  Keep flat (no extra subfolders)
                </span>
              </div>
              {#if recommendedOrg === "flat"}
                <span
                  class="rounded-full border border-[#1FA868] bg-[#E6FBF1] px-2 py-0.5 text-[9px] font-bold uppercase text-[#0E4D31]"
                >
                  Recommended
                </span>
              {/if}
            </div>
            <p class="mt-1 pl-6 text-[11px] text-[#70645D]">
              Writes every renamed file directly into your output directory without creating nested folders.
            </p>
            <div
              class="mt-2 ml-6 space-y-0.5 rounded-md border border-[#E6E0D6] bg-white px-2.5 py-1.5 font-mono text-[10px] text-[#3F3530]"
            >
              {#each sampleItems as sample, idx (`flat-${sample.kind}-${sample.title}-${sample.season ?? 0}-${sample.episode ?? idx}`)}
                <p class="truncate">{formatSamplePath(sample, "flat")}</p>
              {/each}
            </div>
          </button>
        </div>

        <label
          for="remember-folder-org"
          class="flex cursor-pointer items-center gap-2 pt-1 text-xs text-[#61554F]"
        >
          <input
            id="remember-folder-org"
            type="checkbox"
            bind:checked={saveAsDefault}
            class="h-3.5 w-3.5 accent-[#1A1513]"
          />
          <span>
            Remember this choice and don't ask again (changeable anytime in Settings → Metadata &amp; Naming)
          </span>
        </label>
      </div>

      <!-- Footer -->
      <footer
        class="flex items-center justify-between gap-3 border-t border-[#E6E0D6] bg-[#FAF8F4] px-6 py-3.5"
      >
        <button
          type="button"
          onclick={onCancel}
          class="rounded-md border border-[#DAD3C7] bg-white px-3 py-1.5 text-xs font-semibold text-[#61554F] hover:bg-[#EDE7DD] hover:text-[#1A1513]"
        >
          Cancel
        </button>

        <button
          type="button"
          onclick={() => onConfirm(selectedOrg, saveAsDefault)}
          class="rounded-md bg-[#11824F] px-4 py-2 text-xs font-semibold text-white shadow-[0_1px_2px_rgba(17,130,79,0.22)] transition hover:bg-[#0D6E42]"
        >
          Encode {selectedFileCount} {batchKind === "movies" ? (selectedFileCount === 1 ? "Movie" : "Movies") : (selectedFileCount === 1 ? "File" : "Files")} →
        </button>
      </footer>
    </div>
  </div>
{/if}

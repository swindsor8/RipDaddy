<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import type {
    FolderContentKind,
    FolderOrganization,
    NamingConvention,
    OrganizeExecutionResult,
    OrganizeFileRequest,
    OrganizeItemStatus,
    OrganizePlan,
    OrganizePlanRequest,
    OrganizeTransferMode,
  } from "./types";

  interface Props {
    open: boolean;
    items: OrganizeFileRequest[];
    batchKind: FolderContentKind;
    namingConvention: NamingConvention;
    sourceFolderPath: string | null;
    defaultOutputDir: string;
    recommendedOrg: FolderOrganization;
    onCompleted: (result: OrganizeExecutionResult) => void;
    onClose: () => void;
  }

  let {
    open,
    items,
    batchKind,
    namingConvention,
    sourceFolderPath,
    defaultOutputDir,
    recommendedOrg,
    onCompleted,
    onClose,
  }: Props = $props();

  let destinationDir = $state("");
  let folderOrg = $state<FolderOrganization>("showAndSeason");
  let transferMode = $state<OrganizeTransferMode>("move");
  let includeSidecars = $state(true);
  let pruneEmptyDirs = $state(true);

  let plan = $state<OrganizePlan | null>(null);
  let planning = $state(false);
  let planError = $state<string | null>(null);
  let executing = $state(false);
  let executeError = $state<string | null>(null);
  let planSeq = 0;
  let wasOpen = false;

  // Reset modal defaults only on open transition (never mid-session)
  $effect(() => {
    if (open && !wasOpen) {
      const initialDest =
        sourceFolderPath?.trim() || defaultOutputDir.trim() || "";
      destinationDir = initialDest;
      folderOrg =
        batchKind === "movies" && recommendedOrg === "seasonOnly"
          ? "showAndSeason"
          : recommendedOrg;
      transferMode = "move";
      includeSidecars = true;
      pruneEmptyDirs = true;
      planError = null;
      executeError = null;
    }
    wasOpen = open;
  });

  const isInPlace = $derived.by(() => {
    if (!sourceFolderPath || !destinationDir) return false;
    const norm = (s: string) => s.replace(/[\\/]+$/, "").toLowerCase();
    return norm(sourceFolderPath) === norm(destinationDir);
  });

  function buildRequest(): OrganizePlanRequest {
    return {
      items,
      destinationDir: destinationDir.trim(),
      namingConvention,
      folderOrganization: folderOrg,
      transferMode,
      includeSidecars,
      pruneEmptyDirs: transferMode === "move" && pruneEmptyDirs,
      sourceRootDir: sourceFolderPath,
    };
  }

  // Recompute real Rust dry-run plan live whenever any option changes
  $effect(() => {
    if (!open) return;
    const req = buildRequest();
    if (req.items.length === 0 || !req.destinationDir) {
      plan = null;
      planError = req.destinationDir
        ? "Select at least one file to organize."
        : "Choose a target folder to preview paths.";
      return;
    }

    const seq = ++planSeq;
    planning = true;
    planError = null;

    void invoke<OrganizePlan>("plan_organize_folder", { request: req })
      .then((res) => {
        if (seq === planSeq) {
          plan = res;
        }
      })
      .catch((err) => {
        if (seq === planSeq) {
          plan = null;
          planError = err instanceof Error ? err.message : String(err);
        }
      })
      .finally(() => {
        if (seq === planSeq) {
          planning = false;
        }
      });
  });

  async function browseDestination(): Promise<void> {
    try {
      const picked = await openDialog({
        title: "Choose destination folder for organized files",
        directory: true,
        multiple: false,
        defaultPath: destinationDir || sourceFolderPath || undefined,
      });
      if (typeof picked === "string" && picked.trim().length > 0) {
        destinationDir = picked.trim();
      }
    } catch {
      // Ignore canceled dialog
    }
  }

  async function handleExecute(): Promise<void> {
    if (executing || !plan || plan.readyCount === 0) return;
    executing = true;
    executeError = null;
    try {
      const result = await invoke<OrganizeExecutionResult>(
        "execute_organize_folder",
        { request: buildRequest() },
      );
      onCompleted(result);
    } catch (err) {
      executeError = err instanceof Error ? err.message : String(err);
    } finally {
      executing = false;
    }
  }

  function displaySourceLabel(fullPath: string): string {
    if (sourceFolderPath) {
      const prefix = sourceFolderPath.replace(/[\\/]+$/, "");
      if (
        fullPath.toLowerCase().startsWith((prefix + "/").toLowerCase()) ||
        fullPath.toLowerCase().startsWith((prefix + "\\").toLowerCase())
      ) {
        return fullPath.slice(prefix.length + 1);
      }
    }
    const parts = fullPath.split(/[\\/]/);
    return parts.slice(-2).join("/") || fullPath;
  }

  function folderBaseName(path: string): string {
    const trimmed = path.replace(/[\\/]+$/, "");
    const parts = trimmed.split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

  function statusBadgeClass(status: OrganizeItemStatus): string {
    switch (status) {
      case "ready":
        return "border-[#1FA868] bg-[#E6FBF1] text-[#0E4D31]";
      case "unchanged":
        return "border-[#E6E0D6] bg-[#FAF8F4] text-[#7D716A]";
      case "conflict":
      case "missingSource":
        return "border-[#D68D1C] bg-[#FFF7E6] text-[#5E3906]";
    }
  }

  function statusBadgeLabel(status: OrganizeItemStatus): string {
    switch (status) {
      case "ready":
        return transferMode === "move" ? "Rename / Move" : "Copy";
      case "unchanged":
        return "Already Organized";
      case "conflict":
        return "Conflict · Skip";
      case "missingSource":
        return "Missing";
    }
  }

  function handleWindowKeydown(event: KeyboardEvent): void {
    if (open && !executing && event.key === "Escape") {
      onClose();
    }
  }
</script>

<svelte:window onkeydown={handleWindowKeydown} />

{#if open}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4 backdrop-blur-[2px]"
  >
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Review and organize folder"
      class="flex max-h-[88vh] w-full max-w-[780px] flex-col overflow-hidden rounded-2xl border border-[#DAD3C7] bg-white shadow-[0_28px_70px_rgba(26,21,19,0.2)]"
    >
      <div
        class="h-[3px] w-full shrink-0 bg-gradient-to-r from-[#4CDE99] via-[#F8B142] to-[#FA637C]"
        aria-hidden="true"
      ></div>

      <!-- Header -->
      <header class="shrink-0 border-b border-[#E6E0D6] bg-[#FAF8F4] px-6 py-4">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h2 class="text-sm font-extrabold tracking-[-0.02em] text-[#1A1513]">
              Review &amp; Organize Folder
            </h2>
            <p class="mt-0.5 text-xs text-[#61554F]">
              Dry-run preview of every rename, subfolder, and companion subtitle before touching disk.
            </p>
          </div>

          {#if plan}
            <div class="flex flex-wrap items-center gap-1.5 font-mono text-[10px] font-bold">
              <span
                class="rounded-full border border-[#1FA868] bg-[#E6FBF1] px-2.5 py-0.5 text-[#0E4D31]"
              >
                {plan.readyCount} ready
              </span>
              {#if plan.sidecarCount > 0}
                <span
                  class="rounded-full border border-[#9CD9F5] bg-[#E6F6FD] px-2.5 py-0.5 text-[#0E5678]"
                >
                  +{plan.sidecarCount} sidecar{plan.sidecarCount === 1 ? "" : "s"}
                </span>
              {/if}
              {#if plan.unchangedCount > 0}
                <span
                  class="rounded-full border border-[#E6E0D6] bg-white px-2.5 py-0.5 text-[#7D716A]"
                >
                  {plan.unchangedCount} unchanged
                </span>
              {/if}
              {#if plan.conflictCount > 0}
                <span
                  class="rounded-full border border-[#D68D1C] bg-[#FFF7E6] px-2.5 py-0.5 text-[#5E3906]"
                >
                  {plan.conflictCount} conflict{plan.conflictCount === 1 ? "" : "s"}
                </span>
              {/if}
            </div>
          {/if}
        </div>
      </header>

      <!-- Controls Strip -->
      <div class="shrink-0 space-y-3 border-b border-[#EDE7DE] bg-white px-6 py-3.5">
        <!-- Target Folder Picker -->
        <div
          class="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-[#E6E0D6] bg-[#F7F4EE] px-3 py-2 text-[11px]"
        >
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-1.5">
              <span class="font-bold uppercase tracking-[0.06em] text-[#7D716A]">
                Target Folder:
              </span>
              {#if isInPlace}
                <span
                  class="rounded-full border border-[#9BE5C1] bg-[#E6FBF1] px-1.5 py-0.5 text-[9px] font-bold uppercase text-[#0E4D31]"
                >
                  In-Place
                </span>
              {:else}
                <span
                  class="rounded-full border border-[#9CD9F5] bg-[#E6F6FD] px-1.5 py-0.5 text-[9px] font-bold uppercase text-[#0E5678]"
                >
                  Custom Destination
                </span>
              {/if}
            </div>
            <p
              class="mt-0.5 truncate font-mono text-[#1A1513]"
              title={destinationDir}
            >
              {destinationDir || "No folder selected"}
            </p>
          </div>

          <div class="flex shrink-0 items-center gap-1.5">
            {#if sourceFolderPath && !isInPlace}
              <button
                type="button"
                onclick={() => (destinationDir = sourceFolderPath)}
                class="rounded border border-[#9BE5C1] bg-[#E6FBF1] px-2 py-1 text-[10px] font-semibold text-[#0E4D31] hover:bg-[#CEF6E3]"
              >
                In-Place ({folderBaseName(sourceFolderPath)})
              </button>
            {/if}
            <button
              type="button"
              onclick={() => void browseDestination()}
              class="rounded border border-[#DAD3C7] bg-white px-2.5 py-1 text-[10px] font-semibold text-[#1A1513] hover:bg-[#F2EDE5]"
            >
              Change Folder...
            </button>
          </div>
        </div>

        <!-- Folder Layout + Move/Copy + Safety Toggles -->
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div class="flex flex-wrap items-center gap-2">
            <!-- Folder Layout Segmented Control -->
            <div
              class="inline-flex rounded-lg border border-[#DAD3C7] bg-[#FAF8F4] p-0.5 text-[11px]"
              role="group"
              aria-label="Folder structure"
            >
              <button
                type="button"
                onclick={() => (folderOrg = "showAndSeason")}
                class={`rounded-md px-2.5 py-1 font-semibold transition ${
                  folderOrg === "showAndSeason"
                    ? "bg-[#11824F] text-white"
                    : "text-[#61554F] hover:text-[#1A1513]"
                }`}
              >
                {batchKind === "movies" ? "Movie Folders" : "Show & Season"}
              </button>
              {#if batchKind !== "movies"}
                <button
                  type="button"
                  onclick={() => (folderOrg = "seasonOnly")}
                  class={`rounded-md px-2.5 py-1 font-semibold transition ${
                    folderOrg === "seasonOnly"
                      ? "bg-[#11824F] text-white"
                      : "text-[#61554F] hover:text-[#1A1513]"
                  }`}
                >
                  Season Only
                </button>
              {/if}
              <button
                type="button"
                onclick={() => (folderOrg = "flat")}
                class={`rounded-md px-2.5 py-1 font-semibold transition ${
                  folderOrg === "flat"
                    ? "bg-[#11824F] text-white"
                    : "text-[#61554F] hover:text-[#1A1513]"
                }`}
              >
                Flat (Rename Only)
              </button>
            </div>

            <!-- Move vs Copy Toggle -->
            <div
              class="inline-flex rounded-lg border border-[#DAD3C7] bg-[#FAF8F4] p-0.5 text-[11px]"
              role="group"
              aria-label="Transfer mode"
            >
              <button
                type="button"
                onclick={() => (transferMode = "move")}
                class={`rounded-md px-2.5 py-1 font-semibold transition ${
                  transferMode === "move"
                    ? "bg-[#4CDE99] text-[#1A1513]"
                    : "text-[#61554F] hover:text-[#1A1513]"
                }`}
              >
                Move / Rename
              </button>
              <button
                type="button"
                onclick={() => (transferMode = "copy")}
                class={`rounded-md px-2.5 py-1 font-semibold transition ${
                  transferMode === "copy"
                    ? "bg-[#5BC0EB] text-[#1A1513]"
                    : "text-[#61554F] hover:text-[#1A1513]"
                }`}
              >
                Copy
              </button>
            </div>
          </div>

          <!-- Safety Checkboxes -->
          <div class="flex flex-wrap items-center gap-3 text-[11px] text-[#544943]">
            <label class="flex cursor-pointer items-center gap-1.5">
              <input
                type="checkbox"
                bind:checked={includeSidecars}
                class="h-3.5 w-3.5 accent-[#11824F]"
              />
              <span>Move matching <span class="font-mono">.srt/.nfo</span></span>
            </label>
            {#if transferMode === "move"}
              <label class="flex cursor-pointer items-center gap-1.5">
                <input
                  type="checkbox"
                  bind:checked={pruneEmptyDirs}
                  class="h-3.5 w-3.5 accent-[#11824F]"
                />
                <span>Clean empty folders</span>
              </label>
            {/if}
          </div>
        </div>
      </div>

      <!-- Live File-by-File Before -> After Diff List -->
      <div class="min-h-0 flex-1 overflow-y-auto bg-[#FAF8F4] px-6 py-3">
        {#if executeError}
          <div
            class="mb-3 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700"
          >
            {executeError}
          </div>
        {/if}

        {#if planError}
          <div
            class="rounded-lg border border-[#D68D1C] bg-[#FFF7E6] px-3.5 py-2.5 text-xs text-[#5E3906]"
          >
            {planError}
          </div>
        {:else if planning && !plan}
          <div class="py-10 text-center text-xs text-[#7D716A]">
            Computing dry-run rename plan...
          </div>
        {:else if plan}
          <div class="divide-y divide-[#EDE7DD] overflow-hidden rounded-xl border border-[#E6E0D6] bg-white">
            {#each plan.items as entry, idx (`${entry.sourcePath}-${idx}`)}
              <div class="space-y-1 px-3.5 py-2.5 text-xs">
                <div class="flex items-start justify-between gap-2">
                  <div class="min-w-0 flex-1">
                    <p
                      class="truncate font-mono text-[11px] text-[#7D716A]"
                      title={entry.sourcePath}
                    >
                      {displaySourceLabel(entry.sourcePath)}
                    </p>
                    <p
                      class={`mt-0.5 truncate font-mono text-[11px] font-semibold ${
                        entry.status === "ready"
                          ? "text-[#0E4D31]"
                          : entry.status === "unchanged"
                            ? "text-[#7D716A]"
                            : "text-[#5E3906]"
                      }`}
                      title={entry.targetPath}
                    >
                      → {entry.relativeTargetPath}
                    </p>
                  </div>

                  <span
                    class={`shrink-0 rounded-full border px-2 py-0.5 font-mono text-[9px] font-bold uppercase ${statusBadgeClass(entry.status)}`}
                  >
                    {statusBadgeLabel(entry.status)}
                  </span>
                </div>

                {#if entry.sidecars.length > 0}
                  <div class="flex flex-wrap items-center gap-1 pt-0.5">
                    <span class="text-[10px] font-medium text-[#7D716A]">
                      Sidecars:
                    </span>
                    {#each entry.sidecars as sc (sc.sourcePath)}
                      <span
                        title={`${sc.sourcePath} → ${sc.targetPath}`}
                        class="rounded border border-[#9CD9F5] bg-[#E6F6FD] px-1.5 py-0.5 font-mono text-[9px] font-semibold text-[#0E5678]"
                      >
                        +{sc.suffix}
                      </span>
                    {/each}
                  </div>
                {/if}

                {#if entry.reason}
                  <p class="text-[10px] text-[#7D716A]">{entry.reason}</p>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </div>

      <!-- Footer -->
      <footer
        class="flex shrink-0 items-center justify-between gap-3 border-t border-[#E6E0D6] bg-white px-6 py-3.5"
      >
        <p class="text-[11px] text-[#61554F]">
          {#if plan && plan.readyCount > 0}
            Existing files are never overwritten.
            {#if transferMode === "move"}
              One-click Undo is available right after organizing.
            {:else}
              Original source files remain untouched.
            {/if}
          {:else if plan && plan.unchangedCount > 0 && plan.conflictCount === 0}
            Every selected file already matches your naming convention!
          {:else}
            Resolve conflicts or pick another folder to continue.
          {/if}
        </p>

        <div class="flex shrink-0 items-center gap-2">
          <button
            type="button"
            onclick={onClose}
            disabled={executing}
            class="rounded-md border border-[#DAD3C7] bg-white px-3 py-1.5 text-xs font-semibold text-[#61554F] hover:bg-[#EDE7DD] hover:text-[#1A1513] disabled:opacity-50"
          >
            Cancel
          </button>

          <button
            type="button"
            onclick={() => void handleExecute()}
            disabled={executing || planning || !plan || plan.readyCount === 0}
            class="rounded-md bg-[#11824F] px-4 py-2 text-xs font-semibold text-white shadow-[0_1px_2px_rgba(17,130,79,0.22)] transition hover:bg-[#0D6E42] disabled:cursor-not-allowed disabled:bg-[#C6BEB1] disabled:text-[#70645D] disabled:shadow-none"
          >
            {#if executing}
              {transferMode === "move" ? "Organizing..." : "Copying..."}
            {:else if transferMode === "move"}
              Move &amp; Organize {plan?.readyCount ?? 0} File{(plan?.readyCount ?? 0) === 1 ? "" : "s"} →
            {:else}
              Copy &amp; Organize {plan?.readyCount ?? 0} File{(plan?.readyCount ?? 0) === 1 ? "" : "s"} →
            {/if}
          </button>
        </div>
      </footer>
    </div>
  </div>
{/if}

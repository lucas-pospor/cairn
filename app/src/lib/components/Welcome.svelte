<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { app } from "../app.svelte";
  import Icon from "./Icon.svelte";
  import { backend } from "../backend";
  import { isMobile, vaultLabel } from "../platform";

  let typedPath = $state("");
  let appVaults = $state<[string, string][]>([]);

  $effect(() => {
    if (isMobile) backend.appVaults().then((v) => (appVaults = v)).catch(() => {});
  });

  async function createOnDevice() {
    const name = await app.prompt({ title: "Name of the new notebook", value: "Notes", okLabel: "Create notebook" });
    if (name) await app.openVault(name.trim(), true);
  }

  async function pickAndroidFolder() {
    try {
      const r = await backend.pickFolder();
      if (r.uri) await app.openVault(r.uri);
    } catch (e) {
      app.toast(String(e), "error");
    }
  }

  async function openFolder() {
    const dir = await open({ directory: true, multiple: false, title: "Open a folder as a notebook" });
    if (typeof dir === "string") await app.openVault(dir);
  }

  async function createVault() {
    const parentDir = await open({ directory: true, multiple: false, title: "Where should the new notebook go?" });
    if (typeof parentDir !== "string") return;
    const name = await app.prompt({ title: "Name of the new notebook", value: "Notes", okLabel: "Create notebook" });
    if (!name) return;
    const sep = parentDir.includes("\\") && !parentDir.includes("/") ? "\\" : "/";
    await app.openVault(parentDir.replace(/[\\/]+$/, "") + sep + name.trim(), true);
  }

  function baseName(p: string) {
    return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
  }
</script>

<main class="welcome">
  <div class="card">
    <div class="brand">
      <svg viewBox="0 0 1024 1024" width="56" height="56" aria-hidden="true">
        <rect x="64" y="64" width="896" height="896" rx="200" fill="#25282d" />
        <defs>
          <path id="welcome-logo-s0" d="M802 698C800 708 796 719 791 727C787 736 781 743 775 750C769 757 763 763 756 769C749 775 742 781 733 786C724 791 715 797 704 801C693 806 681 810 667 814C653 818 638 821 622 824C605 827 588 829 569 831C550 833 528 834 508 835C488 836 466 837 448 837C429 837 413 837 397 837C381 836 365 836 351 834C336 833 322 831 309 829C296 827 284 824 273 820C262 817 252 813 242 808C233 803 225 798 218 792C210 785 204 779 199 770C193 762 189 751 187 741C184 730 183 719 185 708C187 698 190 689 196 680C203 671 211 663 222 655C232 648 244 641 257 635C270 629 285 624 299 619C313 614 328 610 342 607C357 603 372 599 387 596C402 592 417 589 434 586C451 583 471 580 490 577C509 575 530 572 549 571C568 570 585 569 603 570C620 570 637 571 653 573C670 575 685 578 700 582C715 586 728 590 741 596C753 601 764 608 773 615C782 622 790 630 795 638C800 647 803 656 805 666C806 676 805 687 802 698Z" />
          <path id="welcome-logo-s1" d="M750 524C748 532 744 540 740 546C735 552 729 556 724 560C718 565 711 568 705 571C698 575 691 577 684 580C676 583 669 585 660 587C652 589 644 591 635 593C625 595 615 596 604 597C594 599 583 599 569 600C555 600 538 600 522 599C506 598 488 597 473 596C458 594 445 592 433 590C420 588 408 585 398 582C387 579 377 575 369 571C360 567 353 562 347 558C341 553 337 548 333 543C330 537 328 532 326 526C325 520 325 515 325 508C325 502 327 494 328 487C329 480 331 472 333 466C335 460 338 454 340 449C343 443 347 438 351 433C355 428 360 423 366 419C372 414 380 410 388 407C396 403 406 400 417 398C427 396 439 395 452 395C464 394 477 394 492 395C506 396 524 398 539 400C555 402 571 405 585 408C598 410 609 413 620 416C632 418 642 421 652 425C663 428 673 431 682 435C692 439 701 443 710 447C718 452 726 457 732 462C738 468 744 474 747 480C751 486 753 493 754 500C754 508 753 516 750 524Z" />
          <path id="welcome-logo-s2" d="M661 336C661 342 660 348 659 353C658 358 656 362 654 366C651 370 649 374 645 377C642 381 638 385 634 388C629 391 624 395 618 398C612 401 606 403 598 406C591 408 583 411 575 413C566 415 557 416 547 418C537 420 524 421 512 422C501 423 488 424 478 424C467 425 459 425 450 425C442 424 433 424 426 423C418 423 410 422 403 420C396 419 389 418 383 416C377 414 371 412 366 409C360 406 356 403 352 400C348 396 344 392 342 387C340 382 338 376 338 370C338 364 338 358 340 352C342 347 345 342 348 338C352 333 356 329 361 325C366 321 372 318 378 314C384 311 390 308 397 305C403 302 410 300 417 297C424 295 431 292 439 290C447 288 455 286 464 283C474 281 486 279 497 277C508 275 521 273 532 272C542 271 552 270 562 270C571 270 580 270 589 271C598 272 606 273 613 275C621 277 627 280 633 283C639 285 643 289 647 293C651 296 654 301 656 305C659 309 660 314 661 319C662 324 662 331 661 336Z" />
          <path id="welcome-logo-s3" d="M637 256C635 259 634 263 631 266C629 270 626 273 623 275C620 278 617 280 613 283C609 285 605 287 601 289C597 290 593 292 588 294C583 295 579 296 574 297C568 298 563 299 558 299C552 300 546 300 540 300C534 300 527 300 521 299C515 298 508 297 502 296C496 295 490 293 485 291C479 290 474 288 469 285C464 283 460 281 456 278C451 276 447 273 444 270C440 267 437 264 434 261C431 258 429 255 426 251C424 248 423 244 422 241C420 237 420 233 420 229C420 225 421 221 422 218C423 214 425 211 428 208C431 205 434 202 438 200C441 197 446 195 451 194C455 192 461 191 466 189C471 188 477 188 482 187C488 187 494 187 499 187C505 187 511 187 516 188C522 188 528 189 535 189C541 190 547 191 553 192C558 193 564 194 570 195C575 196 581 198 586 200C592 201 597 203 602 206C607 208 612 210 616 213C621 216 624 219 628 223C631 226 633 229 635 233C637 237 638 240 638 244C638 248 638 252 637 256Z" />
          <mask id="welcome-logo-m0" maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024">
            <rect width="1024" height="1024" fill="#fff" />
            <use href="#welcome-logo-s1" stroke="#000" stroke-width="40" stroke-linejoin="round" />
            <use href="#welcome-logo-s2" stroke="#000" stroke-width="40" stroke-linejoin="round" />
            <use href="#welcome-logo-s3" stroke="#000" stroke-width="40" stroke-linejoin="round" />
          </mask>
          <mask id="welcome-logo-m1" maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024">
            <rect width="1024" height="1024" fill="#fff" />
            <use href="#welcome-logo-s2" stroke="#000" stroke-width="40" stroke-linejoin="round" />
            <use href="#welcome-logo-s3" stroke="#000" stroke-width="40" stroke-linejoin="round" />
          </mask>
          <mask id="welcome-logo-m2" maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024">
            <rect width="1024" height="1024" fill="#fff" />
            <use href="#welcome-logo-s3" stroke="#000" stroke-width="40" stroke-linejoin="round" />
          </mask>
        </defs>
        <g transform="translate(61.44 61.44) scale(0.88)">
          <use href="#welcome-logo-s0" fill="#e06d45" mask="url(#welcome-logo-m0)" />
          <use href="#welcome-logo-s1" fill="#e06d45" mask="url(#welcome-logo-m1)" />
          <use href="#welcome-logo-s2" fill="#e06d45" mask="url(#welcome-logo-m2)" />
          <use href="#welcome-logo-s3" fill="#e06d45" />
        </g>
      </svg>
      <div>
        <h1>Cairn</h1>
        <p class="muted">Your notes are plain Markdown files in a folder you choose.</p>
      </div>
    </div>

    {#if app.opening}
      <p class="opening" role="status" data-testid="vault-opening">Opening {vaultLabel(app.opening)}…</p>
    {/if}

    {#if isMobile}
      <div class="actions mobile">
        <button class="btn primary" onclick={createOnDevice} disabled={!!app.opening} data-testid="create-on-device"><Icon name="folder-plus" /> Create a notebook on this device</button>
        <button class="btn" onclick={pickAndroidFolder} disabled={!!app.opening}><Icon name="folder" /> Open a folder from storage</button>
      </div>
      <p class="hint muted">
        A notebook on this device is private to Cairn and syncs through your Cairn server. A folder from storage can also be
        opened by other apps, but large folders load more slowly.
      </p>
      {#if appVaults.length}
        <h2>On this device</h2>
        <ul class="recent">
          {#each appVaults as [name, path] (path)}
            <li>
              <button class="recent-open" onclick={() => app.openVault(path)} disabled={!!app.opening}>
                <Icon name="vault" />
                <span class="name">{name}</span>
                <span class="path muted"></span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {:else}
    <div class="actions">
      <button class="btn primary" onclick={openFolder} disabled={!!app.opening}><Icon name="folder" /> Open folder as notebook</button>
      <button class="btn" onclick={createVault} disabled={!!app.opening}><Icon name="folder-plus" /> Create new notebook</button>
    </div>

    <form
      class="typed"
      onsubmit={(e) => {
        e.preventDefault();
        if (typedPath.trim()) app.openVault(typedPath.trim(), "ask");
      }}
    >
      <input class="text-input" placeholder="…or type a folder path" aria-label="Folder path" bind:value={typedPath} data-testid="vault-path" />
      <button class="btn" type="submit" disabled={!!app.opening} data-testid="vault-open">Open</button>
    </form>
    {/if}

    {#if app.recent.length}
      <h2>Recent</h2>
      <ul class="recent">
        {#each app.recent as path (path)}
          <li>
            <button class="recent-open" onclick={() => app.openVault(path)} disabled={!!app.opening} title={path}>
              <Icon name="vault" />
              <span class="name">{vaultLabel(path)}</span>
              <span class="path muted">{path.startsWith("content://") ? "folder from storage" : path}</span>
            </button>
            <button class="icon-btn" title="Remove from list" aria-label="Remove {vaultLabel(path)} from list" onclick={() => app.forgetVault(path)}><Icon name="x" size={14} /></button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</main>

<style>
  .welcome {
    height: 100%;
    display: grid;
    place-items: center;
    padding: 16px;
    background: var(--bg-side);
    overflow: auto;
  }
  .card {
    width: min(560px, 100%);
    max-height: 100%;
    overflow: auto;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 32px;
    box-shadow: var(--shadow);
  }
  .brand {
    display: flex;
    gap: 16px;
    align-items: center;
    margin-bottom: 24px;
  }
  h1 {
    margin: 0;
    font-size: 26px;
    letter-spacing: -0.01em;
  }
  .brand p {
    margin: 4px 0 0;
  }
  .opening {
    margin: 0 0 18px;
    font-weight: 600;
  }
  button:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .actions {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }
  .actions.mobile {
    flex-direction: column;
  }
  .actions.mobile .btn {
    justify-content: center;
    padding: 12px;
  }
  .hint {
    font-size: 13px;
    line-height: 1.5;
  }
  .typed {
    display: flex;
    gap: 8px;
    margin-top: 14px;
  }
  h2 {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    margin: 26px 0 8px;
  }
  .recent {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .recent li {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .recent-open {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: 20px auto 1fr;
    align-items: center;
    gap: 8px;
    text-align: left;
    padding: 8px 10px;
    border-radius: var(--radius);
  }
  .recent-open:hover {
    background: var(--bg-hover);
  }
  .name {
    font-weight: 600;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
  }
</style>

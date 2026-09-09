const paths = {
    waveCircle: 'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M6 10v4M9 7v10M12 5v14M15 8v8M18 10v4',
    next: 'M5 5l10 7-10 7zM19 5v14',
    previous: 'M19 5L9 12l10 7zM5 5v14',
    moon: 'M19 15A8 8 0 0 1 9 5a8 8 0 1 0 10 10',
    advance: 'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M8 16l4-9 4 9M10 13h4',
    phone: 'M7 2h10v20H7zM10 19h4',
    eye: 'M2 12q10-13 20 0-10 13-20 0M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',

    bolt: 'M13 2L4 14h7l-1 8 10-13h-8z',
    antenna: 'M9 17a7 7 0 1 1 6 0M10 13a4 4 0 1 1 4 0M12 10v11M9 21h6',
    bars: 'M4 6h16M4 12h16M4 18h16',
    gear: 'M9 3h6l1 4 4 1v7l-4 1-1 4H9l-1-4-4-1V8l4-1zM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
    smile: 'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M8 9h1M15 9h1M8 14q4 5 8 0',

    wave:'M4 10v4M8 5v14M12 2v20M16 6v12M20 9v6',
    account:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M8 18v-2q4-5 8 0v2M15 8a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
    playlist:'M4 5h11M4 9h11M4 13h7M17 4v14M17 4l5-1v5l-5 1M17 18q-6-3-6 1t6-1',
    sort:'M7 3v18M3 7l4-4 4 4M17 21V3M13 17l4 4 4-4',
    filter:'M4 6h16M7 12h10M10 18h4',
    plus:'M12 4v16M4 12h16',
    search:'M20 20l-5-5M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0',
    play:'M9 7l8 5-8 5zM22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0',
    pause:'M8 5v14M16 5v14',
    dots:'M4 12h1M11 12h1M18 12h1',
    queue:'M4 6h16M4 12h12M4 18h8',
    podcasts:'M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z',
    latest:'M12 7v6H8M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0',
    download:'M12 3v12M7 10l5 5 5-5M4 17v4h16v-4',
    more:'M4 12h1M11 12h1M18 12h1',
    back:'M14 5l-7 7 7 7',
    down:'M5 9l7 7 7-7',
    check:'M4 12l5 5L20 6',
    skip:'M5 7a8 8 0 1 1-1 9M5 2v6h6',
    rewind:'M19 7a8 8 0 1 0 1 9M19 2v6h-6',
    speaker:'M4 9h4l5-4v14l-5-4H4zM17 8q5 4 0 8',
    grip:'M8 5h1M15 5h1M8 12h1M15 12h1M8 19h1M15 19h1',
    clock:'M12 7v6l4 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0'
};


const assetRoot = new URL("assets/", document.currentScript.src);

function icon(name) {
    return `<span class="icon"><svg aria-hidden="true" viewBox="0 0 24 24"><path d="${paths[name] || paths.dots}"/></svg></span>`;
}

document.querySelectorAll("[data-icon]").forEach(element => {
    const webIcons = { latest: "bolt", podcasts: "antenna", more: "bars", wave: "smile" };
    const name = element.dataset.icon;
    element.innerHTML = icon(element.closest(".web") ? (webIcons[name] || name) : name);
});

document.querySelectorAll("[data-status]").forEach(element => {
    element.innerHTML = '<div class="status"><span>9:41</span><small><svg width="53" height="13" viewBox="0 0 53 13" aria-hidden="true"><path d="M1 10V8M5 10V6M9 10V3M13 10V1" stroke="white" stroke-width="2"/><path d="M20 4q6-6 12 0M23 7q3-3 6 0M25 10h2" fill="none" stroke="white" stroke-width="1.5"/><rect x="37" y="2" width="13" height="9" rx="2" fill="none" stroke="white"/><rect x="39" y="4" width="9" height="5" fill="white"/><path d="M52 5v3" stroke="white"/></svg></small></div>';
});

const tabs = [
    ["queue", "Queue"],
    ["latest", "Latest"],
    ["podcasts", "Podcasts"],
    ["playlist", "Playlists"],
    ["more", "More"],
];

document.querySelectorAll("[data-tabs]").forEach(element => {
    const active = element.dataset.tabs;
    element.innerHTML = '<nav class="tabs">' + tabs.map(([symbol, label]) =>
        `<span class="tab ${label === active ? "active" : ""}">${icon(symbol)}${label}</span>`
    ).join("") + '</nav><div class="home"></div>';
});

document.querySelectorAll("[data-mini]").forEach(element => {
    const web = element.closest(".web");
    const artwork = web
        ? `<div class="art">${icon("smile")}</div>`
        : `<img class="art" src="${new URL("ferrari.jpg", assetRoot)}" alt="Ferrari artwork">`;
    const title = web ? "FreeBSD with John Baldwin" : "Ferrari";
    const podcast = web ? "Software Engineering Daily" : "Acquired";
    const progress = web ? '<div class="progress"><i></i></div>' : "";
    element.innerHTML = `<div class="mini">${web ? '<span class="close">×</span>' : ""}${artwork}
        <div class="grow"><span class="title">${title}</span><span class="meta">${podcast}</span></div>
        ${progress}${icon("pause")}${web ? "" : icon("skip")}
        ${web ? "" : '<span class="close">×</span>'}</div>`;
});

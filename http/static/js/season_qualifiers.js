"use strict";

function seconds_to_hmmss(secs) {
    let mins = Math.floor(secs / 60);
    let hours = Math.floor(mins / 60);
    let lpad = n => ('0' + n).slice(-2);
    var out = '';
    if (hours > 0) {
        out = `${hours}:${lpad(mins % 60)}:${lpad(secs % 60)}`;
    } else {
        out = `${lpad(mins % 60)}:%{lpad(secs % 60)}`;
    }
    return out;
}

/*
 we're going to trust that the server returns these sorted fastest to slowest
 returns a promise that might be an error
  */
async function get_qualifiers(season_id) {
    return await fetch('/api/v1/season/' + season_id + '/qualifiers')
        .then(r => {
            if (!r.ok) {
                throw new Error("Network error.");
            }
            return r.json();
        }).then(parsed => {
            if (parsed.Err) {
                throw new Error("Server error: " + parsed.Err)
            } else {
                parsed.Ok.forEach(q => {
                    q.time = seconds_to_hmmss(q.time);
                });
                return parsed.Ok;
            }
        });
}

function build_row(template, qual_row, seen) {
    let row = template.content.cloneNode(true).querySelector('tr');
    let cols = row.querySelectorAll('td');
    let [place, player, time, delete_] = cols;
    let name = qual_row.player_name;
    if (seen.players[name]) {
        row.classList.add("hidden", "obsolete-qualifier-times");
        const placeContentContainer = place.querySelector('span');
        placeContentContainer.textContent = "(obsolete)";
    } else {
        const placeContentContainer = place.querySelector('span');
        placeContentContainer.textContent = seen.place;
        seen.place += 1;
        seen.players[name] = true;
    }
    let player_anchor = player.querySelector('a');
    player_anchor.href = "/player/" + name;
    player_anchor.textContent = name;

    let time_anchor = time.querySelector('a');
    time_anchor.href = qual_row.vod;
    time_anchor.textContent = qual_row.time;

    if (delete_) {
        delete_.dataset['target_id'] = qual_row.id;
    }
    return row;
}

function build_rows(qualifiers) {
    const template = document.querySelector('template#qualifier_submission');
    var seen = { place: 1, players: {} };
    let rows = qualifiers.map(e => build_row(template, e, seen));
    return rows;
}

/// deletes the qualifier with this id
/// returns an error if something went wrong, otherwise void
async function do_delete(id) {
    let url = '/api/v1/qualifiers/' + id.toString();
    try {
        let resp = await fetch(url, {
            'method': 'DELETE'
        });
        if (!resp.ok) {
            return 'Error: invalid status: ' + resp.status.toString();
        }
        let res = await resp.json();
        if (res.Err) {
            return res.Err
        }
    } catch (err) {
        console.log("do_delete request error", err);
        return 'Request error: ' + err.toString();
    }
}

document.addEventListener('DOMContentLoaded', async () => {
    let container = document.getElementById('qualifiers');
    let table = document.getElementById('qualifiers_table')
    let season_id = container.dataset['seasonId'];
    let tbody = document.querySelector('#qualifiers_table tbody');
    let toggle_obsolete_button = document.querySelector('button#toggle-obsolete-button');
    let wrapper = document.getElementById('qualifiers-wrapper');
    let no_qualifiers = document.getElementById('no-qualifiers');
    let qualifiers = await get_qualifiers(season_id);
    try {
        var obsolete_hidden = true;
        function rebuild() {
            tbody.innerHTML = "";
            let rows = build_rows(qualifiers);
            if (rows.length == 0) {
                no_qualifiers.classList.remove('hidden');
                wrapper.classList.add('hidden');
                return;
            } else {
                no_qualifiers.classList.add('hidden');
                wrapper.classList.remove('hidden');
            }
            rows.map(r => tbody.appendChild(r));
            let obsolete_rows = document.querySelectorAll('tr.obsolete-qualifier-times');
            toggle_obsolete_button.classList.remove("hidden");
            table.classList.remove('hidden');
            function effect_obsolete_hidden() {
                if (obsolete_hidden) {
                    // hide things
                    obsolete_rows.forEach(r => { r.classList.add('hidden'); });
                    toggle_obsolete_button.textContent = 'Show obsolete';
                } else {
                    // show things
                    obsolete_rows.forEach(r => { r.classList.remove('hidden'); });
                    toggle_obsolete_button.textContent = 'Hide obsolete';
                }
            }
            effect_obsolete_hidden();
            toggle_obsolete_button.addEventListener('click', e => {
                e.preventDefault();
                obsolete_hidden = !obsolete_hidden;
                effect_obsolete_hidden();
            });
        }
        tbody.addEventListener('click', async e => {
            let delete_ = e.target.closest('.delete-qualifier');
            if (!delete_) {
                return;
            }
            let confirmed = confirm("Really delete?");
            if (!confirmed) {
                return;
            }
            let trashcan = delete_.querySelector('img');
            let spinner = delete_.querySelector('.spinner');
            trashcan.classList.add('hidden');
            spinner.classList.remove('hidden');
            let id = parseInt(delete_.dataset['target_id'], 10);
            let res = await do_delete(id);
            spinner.classList.add('hidden');
            trashcan.classList.remove('hidden');
            if (res) {
                alert(res);
            } else {
                // we could have delete_qualifier return an updated list of all qualifiers
                // and rebuild based on that, but i'm choosing to just do all client-side stuff here
                // because adding a returned list to the endpoint seems kind of weird just for this one use case.
                let idx = qualifiers.findIndex(e => { return e.id === id; });
                if (idx !== -1) {
                    qualifiers.splice(idx, 1);
                }
                // this looks like it should be launching an infinite recursion of functions but
                // either it's a big enough stack that it's irrelevant or the async scheduler thingy just solves
                // the problem for us
                rebuild();
            }

        });
        rebuild();
    } catch (e) {
        alert(e);
    }
});

"use strict";

function seconds_to_hmmss(secs) {
    let mins = Math.floor(secs / 60);
    let hours = Math.floor(mins / 60);
    let lpad = n => ('0' + n).slice(-2);
    var out = '';
    if (hours > 0) {
        out = `${hours}:${lpad(mins % 60)}:${lpad(secs % 60)}`;
    }  else {
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
    let [place, player, time, vod, delete_] = cols;
    let name = qual_row.player_name;
    if (seen.players[name]) {
        row.classList.add("hidden", "obsolete", "bg-red-200");
    } else {
        place.textContent = seen.place;
        seen.place += 1;
        seen.players[name] = true;
    }
    player.href = "/player/" + name;
    player.textContent = name;
    time.textContent = qual_row.time;
    let vod_anchor = vod.querySelector('a');
    vod_anchor.href = qual_row.vod;
    vod_anchor.textContent = "link";

    if (delete_) {
        delete_.dataset['target_id'] = qual_row.id;
    }
    return row;
}

function do_delete(id) {
    let url = '/api/v1/qualifiers/' + id.toString();
    return new Promise((res, rej) => {
        fetch(url, {
            'method': 'DELETE'
        }).then(resp => {
            if (resp.ok) {
                resp.json().then(parsed => {
                    if (parsed.Ok !== undefined) {
                        res();
                    } else {
                        rej(parsed.Err || "Unknown or missing error");
                    }
                });
            } else {
                rej("Bad HTTP status: " + resp.status);
            }
        })
    });
}

function build_rows(qualifiers) {
    const template = document.querySelector('template#qualifier_submission');
    var seen = {place: 1, players: {}};
    let rows = qualifiers.map(e => build_row(template, e, seen));
    return rows;
}

document.addEventListener('DOMContentLoaded', async () => {
    let container = document.getElementById('qualifiers');
    let table = document.getElementById('qualifiers_table')
    let season_id = container.dataset['seasonId'];
    let tbody = document.querySelector('#qualifiers_table tbody');
    let qualifiers = await get_qualifiers(season_id).then(async qualifiers => {
        var obsolete_hidden = true;
        function rebuild() {
            console.log("rebuild(): qualifiers.length = " + qualifiers.length.toString());
            tbody.innerHTML = "";
            let rows = build_rows(qualifiers);
            let button = document.querySelector('button#toggle-obsolete');
            rows.map(r => tbody.appendChild(r));
            let obsolete_rows = document.querySelectorAll('tr.obsolete');
            button.classList.remove("hidden");
            table.classList.remove('hidden');
            function effect_obsolete_hidden() {
                if (obsolete_hidden) {
                    // show things
                    obsolete_rows.forEach(r => { r.classList.remove('hidden'); });
                    button.textContent = 'Hide obsolete';
                } else {
                    // hide things
                    obsolete_rows.forEach(r => { r.classList.add('hidden'); });
                    button.textContent = 'Show obsolete';
                }
            }
            effect_obsolete_hidden();
            button.addEventListener('click', e => {
                e.preventDefault();
                effect_obsolete_hidden();
                obsolete_hidden = ! obsolete_hidden;
            });
        }
        tbody.addEventListener('click', async e => {
            let delete_ = e.target.closest('.delete-qualifier');
            if (!delete_) {
                return;
            }

            let trashcan = delete_.querySelector('img');
            let spinner = delete_.querySelector('.spinner');
            trashcan.classList.add('hidden');
            spinner.classList.remove('hidden');
            let id = parseInt(delete_.dataset['target_id'], 10);
            console.log("Deleting " + id);
            await do_delete(id).then(async () => {
                console.log("do_delete succeeded");
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
            }).catch(e => {
                alert("Error deleting qualifier: " + e.toString());
            });
            spinner.classList.add('hidden');
            trashcan.classList.remove('hidden');
        });
        rebuild();

    }).catch(e => {
        let error = document.getElementById('error');
        error.textContent = e;
    });


});
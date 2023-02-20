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
    let name = qual_row.player_name;
    if (seen.players[name]) {
        row.classList.add("hidden", "obsolete", "bg-red-200");
    } else {
        cols[0].textContent = seen.place;
        seen.place += 1;
        seen.players[name] = true;
    }
    let player_name_col = cols[1].querySelector('a');
    player_name_col.href = "/player/" + name;
    player_name_col.textContent = name;
    cols[2].textContent = qual_row.time;
    let vod_anchor = cols[3].querySelector('a');
    vod_anchor.href = qual_row.vod;
    vod_anchor.textContent = "link";
    return row;
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
    let qualifiers = await get_qualifiers(season_id).then(qualifiers => {
        var obsolete_hidden = true;
        let rows = build_rows(qualifiers);
        let button = document.querySelector('button#toggle-obsolete');
        rows.map(r => tbody.appendChild(r));
        let obsolete_rows = document.querySelectorAll('tr.obsolete');
        button.classList.remove("hidden");
        table.classList.remove('hidden');
        button.addEventListener('click', e => {
            e.preventDefault();
            if (obsolete_hidden) {
                // show things
                obsolete_rows.forEach(r => { r.classList.remove('hidden'); });
                button.textContent = 'Hide obsolete';
            } else {
                //hide things
                obsolete_rows.forEach(r => { r.classList.add('hidden'); });
                button.textContent = 'Show obsolete';
            }
            obsolete_hidden = ! obsolete_hidden;
        });
    }).catch(e => {
        let error = document.getElementById('error');
        error.textContent = e;
    });


});
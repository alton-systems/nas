//! Formatação de data/hora compartilhada — usada tanto por
//! `nas-term` (exibição em `Ctrl+t l`/`Ctrl+t a`) quanto por
//! `nas-commands` (nome do checkpoint automático de sessão, ADR 0006
//! §6, etapa 3e). Movida de `nas-term::app` pra aqui pra não duplicar
//! o mesmo algoritmo nos dois lugares.

/// `secs` (desde a época Unix, UTC) como `AAAA-MM-DD HH:MM:SS` —
/// implementado sem nenhuma dependência de data (a única nova que
/// docs/decisions/0005 autoriza é `similar`, pro diff de linhas). O
/// algoritmo (`civil_from_days`) é o de Howard Hinnant pra converter
/// dias desde a época em ano/mês/dia proléptico-gregoriano, amplamente
/// reproduzido e verificado.
pub fn format_timestamp(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let secs_of_day = secs % 86400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_timestamp_bate_com_datas_conhecidas() {
        // conferido com `date -u -d @<segundos>`.
        assert_eq!(format_timestamp(0), "1970-01-01 00:00:00");
        assert_eq!(format_timestamp(1_609_459_200), "2021-01-01 00:00:00");
        assert_eq!(format_timestamp(1_732_000_000), "2024-11-19 07:06:40");
    }
}

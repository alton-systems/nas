# 0002 — Licença MPL-2.0 para o projeto inteiro

## Status

Aceita.

## Contexto

O nas é um projeto open source da organização alton-systems, que depende de
`helix-core` (também MPL-2.0) e, no futuro, de Steel (MPL-2.0). Precisamos de
uma licença que permita uso comercial e privado, mas que proteja mudanças no
próprio código do nas de serem incorporadas de volta sem publicação do
código-fonte das mudanças.

## Decisão

Licenciar todo o repositório sob MPL-2.0 (Mozilla Public License 2.0).

A MPL-2.0 é copyleft por arquivo: qualquer modificação de um arquivo
licenciado sob MPL-2.0 precisa ser publicada sob a mesma licença, mas o
arquivo pode ser combinado com código sob outras licenças (incluindo
proprietárias) num mesmo binário, sem "contaminar" o projeto maior. Isso
alinha com o fato de dependermos de `helix-core` (MPL-2.0) e futuramente de
Steel (MPL-2.0) — mesma família de licença, sem atrito de compatibilidade.

## Consequências

- `LICENSE` na raiz contém o texto integral da MPL-2.0.
- Novos arquivos-fonte devem ser entendidos como licenciados sob MPL-2.0 por
  padrão (não é necessário cabeçalho de licença por arquivo, mas pode ser
  adicionado depois se a organização decidir).
- Dependências de terceiros mantêm sua própria licença; isso não exige
  revisão aqui além de verificar compatibilidade ao adicionar uma dependência
  nova (a maioria do ecossistema Rust usa MIT/Apache-2.0, compatíveis com
  MPL-2.0 em modo de combinação de arquivos, não de projeto).

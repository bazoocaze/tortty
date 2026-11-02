# Backlog — e por que ele é (quase) vazio

**A fila de trabalho deste repositório é [`MIGRATION.md`](MIGRATION.md).** Não
existe uma segunda lista de tarefas: o painel da migração tem o que falta, na
ordem de execução (seção 17), com critério de pronto por fatia. Duas filas
paralelas divergem em uma semana — e aí ninguém sabe qual manda.

O que este arquivo guarda é o que **não é** migração:

- **Melhoria** que vai além da paridade: `MIGRATION.md` seção 16 (M-A … M-O).
- **Limite aceito** ou decisão de não fazer: [`LIMITATIONS.md`](LIMITATIONS.md).
- **Fechado**, com data e verificação: [`HISTORY.md`](HISTORY.md).

## Regras da fila (as mesmas do bzmux Python)

- **Registrar é só registrar.** A investigação (ler código, medir, reproduzir)
  acontece quando o item for executado, não quando é anotado. Na anotação vai o
  sintoma e o que vier de graça no caminho; nada de diagnóstico completo antes
  da hora.
- **Ao fechar um item**: estado para `feito` em `MIGRATION.md`, uma linha
  datada em `HISTORY.md` com o que foi verificado. Item de migração não sai do
  painel — o painel conta o que está pronto, não o que sobrou.
- **Bug corrigido fora de item** entra na lista numerada do `HISTORY.md`; se for
  armadilha de ambiente/ferramenta, entra em [`DEBUG.md`](DEBUG.md).
- **Decisão de não fazer** vai para `LIMITATIONS.md`.

## Aberto fora da fila de migração

Nada. O experimento de viabilidade fechou (fase 1, ver
`TASK-bzmux-in-rust.md`); o que sobrou dele está em `MIGRATION.md`.

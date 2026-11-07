# Rotativo: teste de bancada (Alexandre, JPT M7 + LMC, Windows 32 bits)

Para Luiz conduzir pelo AnyDesk enquanto o Alexandre fica na máquina. O protocolo do eixo (D12) **não foi
verificado** nesta placa. O software não é o dispositivo de segurança: o **E-stop** e a **chave do laser** são.

Regras para todo o teste:

- **E-stop ao alcance da mão do Alexandre** o tempo todo.
- **Chave do laser DESLIGADA** em todos os passos só de eixo (A–C). Só no passo D ela liga, e quem liga é o Alexandre.
- Mãos, cabos e roupas longe do mandril/roletes. O motor pode **terminar o movimento** mesmo depois do Stop do
  software (não existe comando conhecido de "parar eixo"). Para parar o motor já: E-stop, ou desligar a fonte do driver.
- Feche o OpenLaser (janela) antes de usar `openlaser-hw.exe`: os dois não usam o USB ao mesmo tempo.

## 0. Antes de ligar: o que pedir ao Alexandre

1. **Foto das chaves DIP do driver M542C** (SW1–SW8, de perto, legível) **e da etiqueta com a tabela** impressa no
   driver (SW5–8 → pulsos/volta, SW1–3 → corrente). A tabela muda entre versões de driver; vale a etiqueta.
2. **Tipo de fixação:** mandril (castanhas) ou roletes? Se roletes: diâmetro do rolete motriz, em mm (paquímetro).
3. **Relação de transmissão** entre motor e mandril/rolete: acoplado direto (1:1), correia ou engrenagem? Se não for
   direto, contar os dentes das duas polias/engrenagens.
4. **Como o driver está ligado à placa LMC:** foto dos fios PUL+/PUL−, DIR+/DIR−, ENA+/ENA− e de onde chegam na placa
   (conector do eixo de extensão / DB15/DB25). Se ENA não estiver ligado, anotar.
5. **Foto da etiqueta da fonte interna** do gabinete (o driver é alimentado de dentro da torre, sem fonte externa).
   **Tirar o laser da tomada antes de abrir o gabinete.** Ler tensão (24 V?) e corrente máxima.
6. **Foto do conector traseiro do rotativo** (plugue aviação, GX16 ou similar) e do cabo, dos dois lados.
7. Diâmetro do objeto de teste (tubo de sucata), em mm.

Motor (das fotos): 57BYG250C, 1,8°/passo (200 passos/volta), 3,0 A nominal.

### Tabelas do M542C (transcritas da etiqueta do driver do Alexandre)

Corrente, SW1 SW2 SW3 (pico/RMS, A):

| A | SW1 | SW2 | SW3 |   | A | SW1 | SW2 | SW3 |
|---|---|---|---|---|---|---|---|---|
| 1,00/0,71 | on | on | on | | 2,84/2,03 | on | on | off |
| 1,46/1,04 | off | on | on | | 3,31/2,36 | off | on | off |
| 1,91/1,36 | on | off | on | | 3,76/2,69 | on | off | off |
| 2,37/1,69 | off | off | on | | 4,20/3,00 | off | off | off |

SW4: off = meia corrente parado, on = corrente cheia. Para o motor de 3,0 A: **no máximo 4,20/3,00**; 3,76/2,69 ou
3,31/2,36 dá folga térmica e poupa a fonte. Recomendado SW4 = off (meia corrente parado).

Pulsos/volta, SW5 SW6 SW7 SW8:

| pulsos | SW5 | SW6 | SW7 | SW8 |   | pulsos | SW5 | SW6 | SW7 | SW8 |
|---|---|---|---|---|---|---|---|---|---|---|
| 400 | off | on | on | on | | 1000 | on | on | on | off |
| 800 | on | off | on | on | | 2000 | off | on | on | off |
| 1600 | off | off | on | on | | 4000 | on | off | on | off |
| 3200 | on | on | off | on | | 5000 | off | off | on | off |
| 6400 | off | on | off | on | | 8000 | on | on | off | off |
| 12800 | on | off | off | on | | 10000 | off | on | off | off |
| 25600 | off | off | off | on | | 20000 | on | off | off | off |
| | | | | | | 25000 | off | off | off | off |

### Alimentação de dentro do gabinete (hipóteses a confirmar com as fotos 5 e 6)

- Gabinetes de laser fibra costumam ter uma fonte interna de 24 V (muitas vezes Mean Well) que alimenta também a
  placa LMC, e um plugue aviação traseiro (GX16, 4–8 pinos) para o rotativo com 24 V + PUL/DIR/EN.
- Na placa LMCV4 o eixo de extensão sai, em relatos, pelo **CON4 (DB15)**: XPUL pino 15, XDIR pino 14, GND pino 13,
  e um pino de 5 V/Vin que vai a PUL+ e DIR+ (comum positivo; a placa puxa PUL− e DIR−). **Não confirmado** para a
  placa do Alexandre: vale a foto e, se preciso, um multímetro (com a máquina desligada).
- A 24 V o M542C funciona (faixa 24–48 V), mas a velocidade máxima e o torque em velocidade são menores: manter
  velocidade e aceleração conservadoras (100–1000 pulsos/s, 100 ms) até o teste B passar sem perder passo.
- O motor de 3 A precisa caber na sobra da fonte junto com a placa e o galvo. Pela etiqueta da fonte, conferir a
  corrente máxima; se for pequena (ex.: 24 V 2–3 A), usar corrente do driver mais baixa (SW1–3) e SW4 = off.
- O software não depende disso: só os valores padrão e este plano.

## Como calcular os pulsos por volta

`pulsos por volta do mandril = pulsos/volta do driver (SW5–8) × relação de transmissão`

Exemplo: driver em 1600 pulsos/volta, polia do motor 20 dentes e do mandril 60 dentes (3:1) → 1600 × 3 = **4800**.
Acoplado direto → igual ao driver.

Mandril: o desenho envolve o **diâmetro do objeto**. Roletes: envolve o **diâmetro do rolete** (o objeto pode ter
qualquer diâmetro).

## Configurar o perfil (prompt de comando, na pasta do OpenLaser)

```bat
cd /d "%USERPROFILE%\Desktop\OpenLaser-0.1.6-windows-x86"
set OPENLASER_HW_PRESENT=1
openlaser-hw.exe bench start
openlaser-hw.exe profile set rotary.pulses_per_rev 4800
openlaser-hw.exe profile set rotary.kind chuck
openlaser-hw.exe profile set rotary.object_diameter_mm 60
openlaser-hw.exe profile set rotary.axis y
openlaser-hw.exe profile set rotary.max_speed_pps 1000
openlaser-hw.exe profile set rotary.enabled true
openlaser-hw.exe profile show
```

(Cada `set` mostra antigo → novo e pede `SAVE jpt-m7-200`. Os números acima são exemplo: usar os do item 0.)
Velocidade conservadora no início: mínima 100, máxima 1000 pulsos/s, aceleração 100 ms. O mesmo pode ser feito em
**Configurações do dispositivo → Rotativo (eixo estendido)** na janela.

## A. Fotos e valores (sem energia no motor)

- [ ] Fotos do item 0 recebidas; pulsos/volta do driver lidos na etiqueta; relação de transmissão anotada.
- [ ] `profile show` confere: `pulses_per_rev`, `kind`, diâmetros, `enabled: true`.

## B. Jog 90° e 360° (chave do laser DESLIGADA)

Marque com fita uma linha no mandril e outra fixa na base.

```bat
openlaser-hw.exe rotary pos --trace rot-b0.jsonl
openlaser-hw.exe rotary jog 90 --trace rot-b1.jsonl
openlaser-hw.exe rotary jog -90 --trace rot-b2.jsonl
openlaser-hw.exe rotary jog 360 --trace rot-b3.jsonl
openlaser-hw.exe rotary zero --trace rot-b4.jsonl
openlaser-hw.exe rotary pos
```

- [ ] `pos` inicial: anotar o número (esperado 0 após ligar a placa; não verificado).
- [ ] **90°:** girou um quarto de volta? Anotar o **sentido** (visto da ponta do mandril). Se girou ao contrário do
      desenho no passo C, `profile set rotary.reverse true`.
- [ ] **−90°:** voltou à marca.
- [ ] **360°:** a fita voltou exatamente ao mesmo lugar? Se girou menos/mais, corrigir:
      `novo pulses_per_rev = atual × (360 / graus que realmente girou)`.
- [ ] **zero:** voltou à marca e `pos` mostra 0. **O trabalho só começa com a posição em 0.**
- [ ] Teste de Stop (opcional): `rotary jog 720` e apertar Esc no meio. Anotar se o motor **parou** ou **terminou a
      volta**. Esse resultado decide se o Stop do software basta para o eixo (hoje: não garantido).
- [ ] Motor sem perder passo (sem estalos), driver sem alarme (LED vermelho).

## C. Frame no cilindro (só luz vermelha, chave do laser DESLIGADA)

- [ ] Abrir o OpenLaser (`Start OpenLaser with laser.cmd`), conferir **ROTATIVO LIGADO** na barra.
- [ ] Desenho de teste: retângulo de 20 × 60 mm com um texto "F" assimétrico; a dimensão de 60 mm no eixo Y (o eixo
      de giro) envolve o tubo.
- [ ] Frame: o contorno vermelho aparece na linha central do tubo, faixa por faixa, centrado. (O frame não gira o eixo.)
- [ ] As faixas devem ser estreitas **no sentido do giro** (ao redor do tubo). Se ficarem estreitas ao longo do tubo,
      o eixo de giro está trocado: `profile set rotary.axis x` (o perfil usa swapXy).
- [ ] Na confirmação (ARM) conferir: número de faixas, divisão (mm), graus/pulsos totais e se retorna ao início.

## D. Primeira marcação em sucata, potência baixa (o Alexandre inicia)

- [ ] Tubo de sucata no mandril, foco ajustado no topo do tubo, `rotary pos` = 0.
- [ ] Camada a **potência baixa** (ex.: 10–20 %), velocidade normal, uma passada.
- [ ] O Alexandre liga a chave do laser, confere o E-stop, e **ele mesmo** aperta Start / confirma o ARM.
- [ ] Conferir: emendas entre faixas (falha ou sobreposição), sentido do texto, escala (60 mm medidos na
      circunferência), retorno ao início.
- [ ] Guardar os arquivos `rot-*.jsonl` e o log do OpenLaser.

Se as faixas deixam vão ou se sobrepõem: o `pulses_per_rev` ou o diâmetro está errado (passo B). Se o texto sai
espelhado: `rotary.reverse`. Divisão padrão 10 mm; diminuir (`rotary.split_mm 5`) se o foco piorar nas bordas.

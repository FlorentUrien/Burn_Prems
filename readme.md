# Préambule

Le but est de tester la puissance brut du processeur. Il marche sous Linux uniquement (*sinon on risque d'avoir des soucis sur la détermination des coeurs / threads*).

On va donc calculer le nième nombre premier en utilisant entre autre de crible <mark>d'Eratosthenes</mark>, en <mark>Rust</mark>.

# Les fichiers utilisés

![](images/2026-09-30-23-02-36-image.png)

# L'utilisation

Complilation:

`cargo build --release`

Puis lancement :

`./target/release/burn_prems 1_000_000_000 0`

1. Où 1_000_000_000 -> On cherche le miliardième nombre premier

2. 0: Pour lancer sur tous les coeurs / thread, 11: pour lancer sur 1 coeur / 1 thread, 42: pour 4 coeurs avec 2 threads par coeurs...

**<u>Résultats sur mon fixe avec mon i7500 :</u>**

florent@flo-fixe:~/rust/burn_prems`$` ./target/release/burn_prems 1_000_000_000 11

<mark>22801763489, 67.836s, 1 cœur / 1 thread</mark>

florent@flo-fixe:~/rust/burn_prems$ ./target/release/burn_prems 1_000_000_000 0

<mark>22801763489, 18.525s, 4 cœurs / 4 threads (maximum)</mark>

# Les résultats

Sur le miliardième nombre premier.

## Le fixe

22801763489, <mark>67.836s</mark>, 1 cœur / 1 thread

22801763489, <mark>18.525s</mark>, 4 cœurs / 4 threads (maximum)

On note donc une efficacité en MT de <mark>3.66 </mark>soit <mark>91,5%</mark>.

## Le portable

22801763489, <mark>20.149s</mark>, 1 cœur / 1 thread

22801763489, <mark>11.718s</mark>, 1 cœur / 2 threads

22801763489, <mark>11.631s</mark>, 2 cœurs / 2 threads

22801763489, <mark>6.775s</mark>, 4 cœurs / 4 threads

22801763489, <mark>4.298s</mark>, 8 cœurs / 16 threads (maximum)

On note la même efficacité quand les 2 threads tournent sur le même coeur ou qu'un seul sur 2 coeurs différents.
On note aussi une efficacité en MT de <mark>4.69</mark>, soit <mark>29.3%</mark> (*ce qui est faible*).

## Portable vs fixe

Sans surprise le portable malgré sa consommation inférieure gagne nettement.

En mono coeur, mono thread le gain est de <mark>3.37</mark>.

Avec plus de coeurs et de thread malgré une efficacité beaucoup moins grande en MT (42.4% contre 91.5%), l'écart se creuse légèrement à <mark>4.31</mark>.

**<u>Conclusion :</u>** En mono thread le portable écrase le fixe, mais le portable profite assez peu du MT massif car son parallélisme devient moins efficient à cause principalement de latence de l'accès mémoire sur ces cribles qui sont très gourmands en MT.













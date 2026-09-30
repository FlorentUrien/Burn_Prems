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

2.  0: Pour lancer sur tous les coeurs / thread, 11: pour lancer sur 1 coeur / 1 thread, 42: pour 4 coeurs avec 2 threads par coeurs...

**<u>Résultats sur mon fixe avec mon i7500 :</u>**

florent@flo-fixe:~/rust/burn_prems`$` ./target/release/burn_prems 1_000_000_000 11


<mark>22801763489, 67.836s, 1 cœur / 1 thread</mark>


florent@flo-fixe:~/rust/burn_prems$ ./target/release/burn_prems 1_000_000_000 0


<mark>22801763489, 18.525s, 4 cœurs / 4 threads (maximum)</mark>


